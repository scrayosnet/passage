use crate::adapter::authentication::DynAuthenticationAdapter;
use crate::adapter::adapter::Route;
use crate::adapter::discovery::DynDiscoveryActionAdapter;
use crate::adapter::localization::DynLocalizationAdapter;
use crate::adapter::status::DynStatusAdapter;
use crate::error::Result;
use anyhow::{Context, anyhow};
use passage_adapters::{Client, StatusAdapter};
use passage_core::connection::{ConnectionError, Ctx, DispatchError};
use passage_core::packet::handshake::ClientIntentionPacket;
use passage_core::packet::status::{ClientPingRequestPacket, ClientStatusRequestPacket, ServerStatusResponsePacket};
use passage_core::router::Router;
use std::sync::Arc;

/// This crate uses enum dispatch to select the adapters at runtime.
type DynRoute = Route<DynStatusAdapter, DynDiscoveryActionAdapter, DynAuthenticationAdapter, DynLocalizationAdapter>;

/// This crate uses enum dispatch to select the adapters at runtime.
type DynRoutes = Arc<[DynRoute]>;

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
    client: Client,

    /// The current step of the connection. This binds the client to the server protocol.
    step: Step,
}

impl State {
    pub fn new(routes: DynRoutes) -> Self {
        Self {
            routes,
            route_index: None,
            client: Default::default(),
            step: Step::Intention,
        }
    }

    pub fn find_route(&mut self, server_address: &str) {
        self.route_index =  self.routes
            .iter()
            .enumerate()
            .find(|(_, route)| route.hostname.is_match(server_address))
            .map(|(index, _)| index);
    }

    pub fn route(&self) -> Option<&DynRoute> {
        self.route_index.map(|index| &self.routes[index])
    }
}

pub struct Passage {}

impl Passage {
    pub fn new() -> Self {
        Self {}
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

    pub async fn serve(mut self) -> Result<()> {
        let router = self.router();
        Ok(())
    }
}

// handlers

fn on_open(ctx: Ctx<'_, State>) -> Result<(), DispatchError> {
    Ok(())
}

fn on_tick(ctx: Ctx<'_, State>) -> Result<(), DispatchError> {
    Ok(())
}

fn on_error(ctx: Ctx<'_, State>, error: &mut ConnectionError) -> Result<(), DispatchError> {
    Ok(())
}

fn on_handshake_intention_packet(ctx: Ctx<'_, State>, packet: ClientIntentionPacket) -> Result<(), DispatchError> {
    // Ensure that the connection is in the `intention` state.
    if ctx.state.step != Step::Intention {
        return Err(DispatchError::peer(
            "unexpected_step",
            anyhow!("Expected step `intention`, got `{:?}`", ctx.state.step),
        ));
    }

    // Calculate the next step the connection should take. Then send the state update.
    let next_step = match packet.next_state {
        passage_core::common::State::Status => Step::StatusPingRequest,
        passage_core::common::State::Login => Step::StatusPingRequest,
        passage_core::common::State::Transfer => Step::StatusPingRequest,
    };
    ctx.handle.update(move |state| {
        state.find_route(&packet.server_address);
        state.step = next_step;
        state.client.protocol_version = packet.protocol_version;
        state.client.server_address = packet.server_address;
        state.client.server_port = packet.server_port;
    })?;
    Ok(())
}

fn on_status_request_packet(ctx: Ctx<'_, State>, packet: ClientStatusRequestPacket) -> Result<(), DispatchError> {
    // Ensure that the connection is in the `status_request` state.
    if ctx.state.step != Step::StatusRequest {
        return Err(DispatchError::peer(
            "unexpected_step",
            anyhow!("Expected step `status_request`, got `{:?}`", ctx.state.step),
        ));
    }

    // TODO maybe handle the error directly? Close connection or send some status?
    // Get the status adapter form the current route.
    let route = ctx.state.route()
        .ok_or_else(|| DispatchError::peer("no_route", anyhow!("No route found for the client")))?;
    let version = ctx.version;
    let handle = ctx.handle.clone();
    let client = ctx.state.client.clone();
    handle.exclusive(async move {
        let status = route.status(&client)
            .await
            .context("failed to get status")?
            .ok_or_else(|| DispatchError::internal("no_status", anyhow!("No status found for the client")))?;
        handle.send(version, ServerStatusResponsePacket::try_from(&status).context("")?)?;
        Ok(())
    })?;
    Ok(())
}

fn on_status_ping_request_packet(ctx: Ctx<'_, State>, packet: ClientPingRequestPacket) -> Result<(), DispatchError> {
    Ok(())
}
