fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure()
        .protoc_arg("--experimental_allow_proto3_optional")
        .build_server(false)
        .type_attribute(".", "#[derive(serde::Serialize,serde::Deserialize)]")
        .compile_protos(
            &[
                "proto/adapter/adapter.proto",
                "proto/adapter/authentication.proto",
                "proto/adapter/discovery.proto",
                "proto/adapter/localization.proto",
                "proto/adapter/status.proto",
                "proto/adapter/discovery_action.proto",
            ],
            &["proto"],
        )?;
    Ok(())
}
