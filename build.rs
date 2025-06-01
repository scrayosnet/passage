fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "grpc")]
    tonic_build::configure()
        .protoc_arg("--experimental_allow_proto3_optional")
        .build_server(false)
        .type_attribute(".", "#[derive(serde::Serialize,serde::Deserialize)]")
        .compile_protos(
            &[
                "proto/adapter/adapter.proto",
                "proto/adapter/discovery.proto",
                "proto/adapter/resourcepack.proto",
                "proto/adapter/status.proto",
                "proto/adapter/strategy.proto",
            ],
            &["proto"],
        )?;
    Ok(())
}
