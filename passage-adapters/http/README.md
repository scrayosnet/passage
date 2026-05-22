# HTTP Adapter

This crate provides HTTP-based adapters. It contains the `MojangAdapter` for official Minecraft authentication
and the `HttpStatusAdapter` for polling a remote HTTP endpoint for server status.

## Mojang Authentication Adapter

The Mojang authentication adapter implements the official authentication mechanism. It makes an HTTP request to the Mojang API, verifying that the client has requested to join the server.

Generally, this adapter will be used.

## Http Status Adapter

The HTTP status adapter makes periodic requests to an HTTP server which serves the status for Passage. This refresh is fully disconnected from the target selection such that refreshes do not block the player status flow.
