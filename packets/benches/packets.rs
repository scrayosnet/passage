use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use packets::{
    ChatMode, DisplayedSkinParts, MainHand, ParticleStatus, ReadPacket, ResourcePackResult, State,
    WritePacket, configuration, handshake, login, status,
};
use std::fmt::Debug;
use std::io::Cursor;
use uuid::uuid;

pub async fn rw_packet<T>(packet: T, buf: Vec<u8>)
where
    T: PartialEq + Eq + ReadPacket + WritePacket + Send + Sync + Debug + Clone,
{
    // write packets
    let mut writer: Cursor<Vec<u8>> = Cursor::new(buf);
    packet
        .write_to_buffer(&mut writer)
        .await
        .expect("failed to write packets");

    // read packets
    let mut reader: Cursor<Vec<u8>> = Cursor::new(writer.into_inner());
    T::read_from_buffer(&mut reader)
        .await
        .expect("failed to read packets");
}

fn criterion_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("rw");
    let runtime = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    let user_id = uuid!("09879557-e479-45a9-b434-a56377674627");
    let packet_id = uuid!("9c09eef4-f68d-4387-9751-72bbff53d5a0");
    let buf = Vec::with_capacity(1000);

    group.bench_function(
        BenchmarkId::new("handshake::serverbound::HandshakePacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    handshake::serverbound::HandshakePacket {
                        protocol_version: 742,
                        server_address: "mc.justchunks.net".to_string(),
                        server_port: 26426,
                        next_state: State::Status,
                    },
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("status::clientbound::StatusResponsePacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    status::clientbound::StatusResponsePacket {
                        body: "JustChunks".to_string(),
                    },
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("status::clientbound::PongPacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    status::clientbound::PongPacket { payload: 100 },
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("status::serverbound::StatusRequestPacket", 0),
        |b| {
            b.to_async(&runtime)
                .iter(|| rw_packet(status::serverbound::StatusRequestPacket, buf.clone()))
        },
    );

    group.bench_function(
        BenchmarkId::new("status::serverbound::PingPacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    status::serverbound::PingPacket { payload: 100 },
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("login::clientbound::DisconnectPacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    login::clientbound::DisconnectPacket {
                        reason: "kicked".to_string(),
                    },
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("login::clientbound::EncryptionRequestPacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    login::clientbound::EncryptionRequestPacket {
                        server_id: "mc.justchunks.net".to_string(),
                        public_key: vec![0u8; 32],
                        verify_token: [0u8; 32],
                        should_authenticate: false,
                    },
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("login::clientbound::LoginSuccessPacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    login::clientbound::LoginSuccessPacket {
                        user_id,
                        user_name: "Hydrofin".to_string(),
                    },
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("login::clientbound::SetCompressionPacket", 0),
        |b| {
            b.to_async(&runtime)
                .iter(|| rw_packet(login::clientbound::SetCompressionPacket, buf.clone()))
        },
    );

    group.bench_function(
        BenchmarkId::new("login::clientbound::LoginPluginRequestPacket", 0),
        |b| {
            b.to_async(&runtime)
                .iter(|| rw_packet(login::clientbound::LoginPluginRequestPacket, buf.clone()))
        },
    );

    group.bench_function(
        BenchmarkId::new("login::clientbound::CookieRequestPacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    login::clientbound::CookieRequestPacket {
                        key: "passage:something".to_string(),
                    },
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("login::serverbound::LoginStartPacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    login::serverbound::LoginStartPacket {
                        user_id,
                        user_name: "Hydrofin".to_string(),
                    },
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("login::serverbound::EncryptionResponsePacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    login::serverbound::EncryptionResponsePacket {
                        shared_secret: vec![0u8; 32],
                        verify_token: vec![0u8; 32],
                    },
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("login::serverbound::LoginPluginResponsePacket", 0),
        |b| {
            b.to_async(&runtime)
                .iter(|| rw_packet(login::serverbound::LoginPluginResponsePacket, buf.clone()))
        },
    );

    group.bench_function(
        BenchmarkId::new("login::serverbound::LoginAcknowledgedPacket", 0),
        |b| {
            b.to_async(&runtime)
                .iter(|| rw_packet(login::serverbound::LoginAcknowledgedPacket, buf.clone()))
        },
    );

    group.bench_function(
        BenchmarkId::new("login::serverbound::CookieResponsePacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    login::serverbound::CookieResponsePacket {
                        key: "passage:something".to_string(),
                        payload: Some(vec![0u8; 16]),
                    },
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("configuration::clientbound::CookieRequestPacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    configuration::clientbound::CookieRequestPacket {
                        key: "foo".to_string(),
                    },
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("configuration::clientbound::PluginMessagePacket", 0),
        |b| {
            b.to_async(&runtime)
                .iter(|| rw_packet(configuration::clientbound::PluginMessagePacket, buf.clone()))
        },
    );

    group.bench_function(
        BenchmarkId::new("configuration::clientbound::DisconnectPacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    configuration::clientbound::DisconnectPacket {
                        reason: "kicked".to_string(),
                    },
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("configuration::clientbound::FinishConfigurationPacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    configuration::clientbound::FinishConfigurationPacket,
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("configuration::clientbound::KeepAlivePacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    configuration::clientbound::KeepAlivePacket { id: 100 },
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("configuration::clientbound::PingPacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    configuration::clientbound::PingPacket { id: 100 },
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("configuration::clientbound::ResetChatPacket", 0),
        |b| {
            b.to_async(&runtime)
                .iter(|| rw_packet(configuration::clientbound::ResetChatPacket, buf.clone()))
        },
    );

    group.bench_function(
        BenchmarkId::new("configuration::clientbound::RegistryDataPacket", 0),
        |b| {
            b.to_async(&runtime)
                .iter(|| rw_packet(configuration::clientbound::RegistryDataPacket, buf.clone()))
        },
    );

    group.bench_function(
        BenchmarkId::new("configuration::clientbound::RemoveResourcePackPacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    configuration::clientbound::RemoveResourcePackPacket,
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("configuration::clientbound::AddResourcePackPacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    configuration::clientbound::AddResourcePackPacket {
                        uuid: packet_id,
                        url: "https://impackable.justchunks.net/download/67e3e6e8704c701ec3cf5f8b"
                            .to_string(),
                        hash: "c7affa49facf2b14238f1d2f7f04d7d0360bdb1d".to_string(),
                        forced: true,
                        prompt_message: Some("Please install!".to_string()),
                    },
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("configuration::clientbound::StoreCookiePacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    configuration::clientbound::StoreCookiePacket {
                        key: "passage:something".to_string(),
                        payload: vec![0u8; 16],
                    },
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("configuration::clientbound::TransferPacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    configuration::clientbound::TransferPacket {
                        host: "mc.justchunks.net".to_string(),
                        port: 25565,
                    },
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("configuration::clientbound::FeatureFlagsPacket", 0),
        |b| {
            b.to_async(&runtime)
                .iter(|| rw_packet(configuration::clientbound::FeatureFlagsPacket, buf.clone()))
        },
    );

    group.bench_function(
        BenchmarkId::new("configuration::clientbound::UpdateTagsPacket", 0),
        |b| {
            b.to_async(&runtime)
                .iter(|| rw_packet(configuration::clientbound::UpdateTagsPacket, buf.clone()))
        },
    );

    group.bench_function(
        BenchmarkId::new("configuration::clientbound::KnownPacksPacket", 0),
        |b| {
            b.to_async(&runtime)
                .iter(|| rw_packet(configuration::clientbound::KnownPacksPacket, buf.clone()))
        },
    );

    group.bench_function(
        BenchmarkId::new("configuration::clientbound::CustomReportDetailsPacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    configuration::clientbound::CustomReportDetailsPacket,
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("configuration::clientbound::ServerLinksPacket", 0),
        |b| {
            b.to_async(&runtime)
                .iter(|| rw_packet(configuration::clientbound::ServerLinksPacket, buf.clone()))
        },
    );

    group.bench_function(
        BenchmarkId::new("configuration::serverbound::ClientInformationPacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    configuration::serverbound::ClientInformationPacket {
                        locale: "de".to_string(),
                        view_distance: 12,
                        chat_mode: ChatMode::Enabled,
                        chat_colors: false,
                        displayed_skin_parts: DisplayedSkinParts(0),
                        main_hand: MainHand::Left,
                        enable_text_filtering: false,
                        allow_server_listing: false,
                        particle_status: ParticleStatus::All,
                    },
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("configuration::serverbound::CookieResponsePacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    configuration::serverbound::CookieResponsePacket,
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("configuration::serverbound::PluginMessagePacket", 0),
        |b| {
            b.to_async(&runtime)
                .iter(|| rw_packet(configuration::serverbound::PluginMessagePacket, buf.clone()))
        },
    );

    group.bench_function(
        BenchmarkId::new(
            "configuration::serverbound::AckFinishConfigurationPacket",
            0,
        ),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    configuration::serverbound::AckFinishConfigurationPacket,
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("configuration::serverbound::KeepAlivePacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    configuration::serverbound::KeepAlivePacket { id: 420 },
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("configuration::serverbound::PongPacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    configuration::serverbound::PongPacket { id: 420 },
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("configuration::serverbound::ResourcePackResponsePacket", 0),
        |b| {
            b.to_async(&runtime).iter(|| {
                rw_packet(
                    configuration::serverbound::ResourcePackResponsePacket {
                        uuid: packet_id,
                        result: ResourcePackResult::Success,
                    },
                    buf.clone(),
                )
            })
        },
    );

    group.bench_function(
        BenchmarkId::new("configuration::serverbound::KnownPacksPacket", 0),
        |b| {
            b.to_async(&runtime)
                .iter(|| rw_packet(configuration::serverbound::KnownPacksPacket, buf.clone()))
        },
    );
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
