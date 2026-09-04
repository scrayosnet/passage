These changes should be applied to the driver:
- The erased packet should be removed. The version will not change after the handshake. Instead, it should send the encoded packet with a version and phase guard to ensure that it is only sent for that configuration. Sending the bytes view should also perform better (small clone).
- The three-staged setup could probably be flattened (into two or even one): Shared -> ConnHandle -> Ctx
- The cipher should be templated as it is called often
