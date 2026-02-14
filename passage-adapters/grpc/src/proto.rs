use crate::error::MissingFieldError;
use passage_adapters::{Error, ServerPlayer, ServerPlayers, ServerStatus, ServerVersion};
use serde_json::value::RawValue;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

tonic::include_proto!("scrayosnet.passage.adapter");

impl From<&passage_adapters::Target> for Target {
    fn from(value: &passage_adapters::Target) -> Self {
        Self {
            identifier: value.identifier.clone(),
            address: Some(Address {
                hostname: value.address.ip().to_string(),
                port: u32::from(value.address.port()),
            }),
            meta: value
                .meta
                .iter()
                .map(|(k, v)| MetaEntry {
                    key: k.clone(),
                    value: v.clone(),
                })
                .collect(),
        }
    }
}

impl TryFrom<Target> for passage_adapters::Target {
    type Error = passage_adapters::Error;

    fn try_from(value: Target) -> Result<Self, Self::Error> {
        let Some(raw_addr) = value.address.clone() else {
            return Err(passage_adapters::Error::FailedParse {
                adapter_type: "grpc",
                cause: Box::new(MissingFieldError { field: "address" }),
            });
        };
        let address = SocketAddr::from_str(&format!("{}:{}", raw_addr.hostname, raw_addr.port))
            .map_err(|err| passage_adapters::Error::FailedParse {
                adapter_type: "grpc",
                cause: err.into(),
            })?;

        Ok(Self {
            identifier: value.identifier,
            address,
            meta: value
                .meta
                .into_iter()
                .map(|entry| (entry.key, entry.value))
                .collect(),
        })
    }
}

impl TryFrom<StatusData> for ServerStatus {
    type Error = Error;

    fn try_from(value: StatusData) -> Result<Self, Self::Error> {
        let description = value
            .description
            .map(RawValue::from_string)
            .transpose()
            .map_err(|err| Error::FailedParse {
                adapter_type: "grpc",
                cause: err.into(),
            })?;

        let favicon = value
            .favicon
            .map(String::from_utf8)
            .transpose()
            .map_err(|err| Error::FailedParse {
                adapter_type: "grpc",
                cause: err.into(),
            })?;

        Ok(Self {
            version: value.version.map(Into::into).ok_or(Error::FailedParse {
                adapter_type: "grpc",
                cause: Box::new(MissingFieldError {
                    field: "status.version",
                }),
            })?,
            players: value.players.map(Into::into),
            description,
            favicon,
            enforces_secure_chat: value.enforces_secure_chat,
        })
    }
}

impl From<ProtocolVersion> for ServerVersion {
    fn from(value: ProtocolVersion) -> Self {
        Self {
            name: value.name,
            protocol: value.protocol,
        }
    }
}

impl From<Players> for ServerPlayers {
    fn from(value: Players) -> Self {
        let samples: Option<Vec<ServerPlayer>> = if value.samples.is_empty() {
            None
        } else {
            Some(
                value
                    .samples
                    .iter()
                    .map(|raw| ServerPlayer {
                        name: raw.name.clone(),
                        id: raw.id.clone(),
                    })
                    .collect(),
            )
        };

        Self {
            online: value.online,
            max: value.max,
            sample: samples,
        }
    }
}

impl TryFrom<Address> for SocketAddr {
    type Error = Error;

    fn try_from(value: Address) -> Result<Self, Self::Error> {
        Ok(Self::new(
            IpAddr::from_str(&value.hostname).map_err(|err| Error::FailedParse {
                adapter_type: "grpc",
                cause: err.into(),
            })?,
            u16::try_from(value.port).map_err(|err| Error::FailedParse {
                adapter_type: "grpc",
                cause: err.into(),
            })?,
        ))
    }
}
