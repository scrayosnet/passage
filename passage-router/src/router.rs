use tokio::net::TcpListener;
use crate::adapter::adapter::Route;
use crate::adapter::authentication::DynAuthenticationAdapter;
use crate::adapter::discovery::DynDiscoveryActionAdapter;
use crate::adapter::localization::DynLocalizationAdapter;
use crate::adapter::status::DynStatusAdapter;
use crate::error::Result;
use anyhow::{Context, anyhow};
use passage_adapters::{Client, ServerStatus, StatusAdapter};
use passage_core::connection::{Conn, ConnRef, ConnectionError, DispatchError};
use passage_core::packet::handshake::ClientIntentionPacket;
use passage_core::packet::status::{ClientPingRequestPacket, ClientStatusRequestPacket, ServerPongResponsePacket, ServerStatusResponsePacket};
use passage_core::router::Router;
use std::sync::Arc;
use passage_core::common;
use passage_core::server::{Listener, Server};

/// This crate uses enum dispatch to select the adapters at runtime.
type DynRoute = Route<
    DynStatusAdapter,
    DynDiscoveryActionAdapter,
    DynAuthenticationAdapter,
    DynLocalizationAdapter,
>;

/// This crate uses enum dispatch to select the adapters at runtime.
///
/// The inner `Arc` is what lets a handler carry one route into an adapter call: it has to survive
/// the `.await`, and a borrow of the state cannot.
type DynRoutes = Arc<[Arc<DynRoute>]>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    Intention,
    StatusRequest,
    StatusPingRequest,
    Completed,
}

pub struct State {
    /// The shared set of routes.
    routes: DynRoutes,

    /// The index of the currently selected route.
    route_index: Option<usize>,

    /// The client information.
    ///
    /// Behind an `Arc` because every adapter call needs it and none of them may borrow the state:
    /// a handler holds the connection only for the body of a closure, never across an `.await`.
    client: Arc<Client>,

    /// The current step of the connection. This binds the client to the server protocol.
    step: Step,
}

impl State {
    pub fn new(routes: DynRoutes, address: std::net::SocketAddr) -> Self {
        let mut client = Client::default();
        client.address = address;
        Self {
            routes,
            route_index: None,
            client: Arc::new(client),
            step: Step::Intention,
        }
    }

    pub fn find_route(&mut self, server_address: &str) {
        self.route_index = self
            .routes
            .iter()
            .enumerate()
            .find(|(_, route)| route.hostname.is_match(server_address))
            .map(|(index, _)| index);
    }

    /// The selected route, as something a handler can carry into an adapter call.
    pub fn route(&self) -> Option<Arc<DynRoute>> {
        self.route_index
            .map(|index| Arc::clone(&self.routes[index]))
    }

    /// Mutable access to the client information, which is shared with in-flight adapter calls.
    fn client_mut(&mut self) -> &mut Client {
        Arc::make_mut(&mut self.client)
    }
}

pub struct Passage {
    routes: DynRoutes,
}

impl Passage {
    pub fn new(routes: DynRoutes) -> Self {
        Self { routes }
    }

    // TODO pass adapters or config?

    fn router(&self) -> Result<Arc<Router<State>>> {
        let router = Router::builder()
            // general handlers
            .on_open(on_open)
            .on_tick(on_tick)
            .on_error(on_error)
            // packet handlers
            .on(on_handshake_intention_packet)?
            .on(on_status_request_packet)?
            .on(on_status_ping_request_packet)?
            .build();
        Ok(Arc::new(router))
    }

    pub async fn serve(self) -> Result<()> {
        let router = self.router()?;
        let listener = TcpListener::bind("").await?;
        Server::new(listener)
            .dispatch(router)
            .state(move |addr| State::new(self.routes.clone(), addr.clone()))
            .serve()
            .await;
        Ok(())
    }
}

// handlers

fn on_open(conn: ConnRef<'_, State>) -> Result<(), DispatchError> {
    let _ = conn;
    Ok(())
}

async fn on_tick(conn: ConnRef<'_, State>) -> Result<(), DispatchError> {
    let _ = conn;
    Ok(())
}

fn on_error(conn: ConnRef<'_, State>, error: &mut ConnectionError) -> Result<(), DispatchError> {
    let _ = (conn, error);
    Ok(())
}

/// Fails unless the connection is at the step this handler answers.
///
/// The step and the route are written together and read together, which is the reason they are
/// plain fields under one lend rather than separately shared: a caller must never see the step
/// advanced while the route it implies is still missing.
fn expect_step(conn: &Conn<State>, step: Step) -> Result<(), DispatchError> {
    if conn.state.step == step {
        return Ok(());
    }
    Err(DispatchError::peer(
        "unexpected_step",
        anyhow!("Expected step `{step:?}`, got `{:?}`", conn.state.step),
    ))
}

async fn get_status(route: Option<&Arc<DynRoute>>, client: &Client) -> Result<ServerStatus, DispatchError> {
    let Some(route) = route else {
        return Err(DispatchError::peer("no_route", anyhow!("No route found")));
    };
    route.status_adapter
        .status(client)
        .await
        .map_err(|err| DispatchError::internal("status_error", err))?
        .ok_or(DispatchError::internal("no_status", anyhow!("No status found for the client")))
}

async fn on_handshake_intention_packet(
    conn: ConnRef<'_, State>,
    packet: ClientIntentionPacket,
) -> Result<(), DispatchError> {
    conn.with(|c| {
        expect_step(c, Step::Intention)?;
        c.state.find_route(&packet.server_address);
        c.state.step = match packet.next_state {
            common::State::Status => Step::StatusRequest,
            common::State::Login => Step::StatusPingRequest,
            common::State::Transfer => Step::StatusPingRequest,
        };
        let client = c.state.client_mut();
        client.protocol_version = packet.protocol_version;
        client.server_address = packet.server_address;
        client.server_port = packet.server_port;
        Ok(())
    })
}

async fn on_status_request_packet(
    conn: ConnRef<'_, State>,
    _packet: ClientStatusRequestPacket,
) -> Result<(), DispatchError> {
    let (route, client) = conn.with(|c| {
        expect_step(c, Step::StatusRequest)?;
        Ok::<_, DispatchError>((c.state.route(), Arc::clone(&c.state.client)))
    })?;
    let status = get_status(route.as_ref(), &client).await?;
    let packet = ServerStatusResponsePacket::try_from(&status)
        .map_err(|err| DispatchError::internal("status_encode_error", err))?;
    conn.send(packet)?;
    Ok(())
}

async fn on_status_ping_request_packet(
    conn: ConnRef<'_, State>,
    packet: ClientPingRequestPacket,
) -> Result<(), DispatchError> {
    conn.send(ServerPongResponsePacket { payload: packet.payload })?;
    conn.close();
    Ok(())
}
