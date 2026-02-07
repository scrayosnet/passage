---
title: Authentication and Encryption
description: Understanding how Passage authenticates players and encrypts connections.
---

Passage implements Minecraft's standard authentication and encryption protocol to ensure only legitimate players can connect and all data is encrypted. This page explains the technical details of this process.

## Overview

Every player connection goes through these security phases:

1. **RSA Key Exchange**: Passage generates and sends an RSA public key
2. **Client Authentication**: Client authenticates with Mojang's session servers
3. **Shared Secret Exchange**: Client generates and encrypts a shared secret
4. **Server Verification**: Passage verifies with Mojang that the client authenticated
5. **Connection Encryption**: All further communication is encrypted with AES-128-CFB8
6. **Optional Cookie**: Passage can issue an authentication cookie for fast re-authentication

## The Authentication Flow

### Step 1: Initial Handshake

```
Player → Passage: Handshake (protocol version, server address)
Player → Passage: Login Start (username)
```

The player sends their username (e.g., "Steve") to initiate login.

### Step 2: Encryption Request

Passage generates cryptographic keys and sends an encryption request:

```rust
// Passage generates a 1024-bit RSA keypair at startup
let (private_key, public_key) = generate_rsa_keypair();

// Encode public key in DER format for Minecraft protocol
let encoded_public_key = public_key.to_public_key_der();

// Generate random 32-byte verify token
let verify_token = random_bytes(32);
```

```
Passage → Player: Encryption Request {
    server_id: "",  // Always empty string
    public_key: <RSA public key in DER format>,
    verify_token: <32 random bytes>
}
```

### Step 3: Client-Side Authentication

The client performs several operations:

1. **Generate Shared Secret**: 16 random bytes
2. **Calculate Server Hash**: Special Minecraft SHA-1 hash
3. **Authenticate with Mojang**: POST to sessionserver.mojang.com
4. **Encrypt Data**: Encrypt shared secret and verify token with Passage's public key

```
Client calculates:
server_hash = minecraft_sha1(server_id + shared_secret + public_key)

Client sends to Mojang:
POST https://sessionserver.mojang.com/session/minecraft/join
{
  "accessToken": "<player's session token>",
  "selectedProfile": "<player's UUID>",
  "serverId": "<server_hash>"
}
```

If Mojang returns 204 No Content, authentication succeeded.

### Step 4: Encryption Response

Client sends encrypted data to Passage:

```
Player → Passage: Encryption Response {
    shared_secret: RSA_encrypt(shared_secret, public_key),
    verify_token: RSA_encrypt(verify_token, public_key)
}
```

### Step 5: Server-Side Verification

Passage verifies the client's authentication:

```rust
// Decrypt shared secret and verify token with private key
let shared_secret = rsa_decrypt(encrypted_shared_secret, private_key);
let decrypted_token = rsa_decrypt(encrypted_verify_token, private_key);

// Verify token matches what we sent
assert_eq!(verify_token, decrypted_token);

// Calculate same server hash
let server_hash = minecraft_sha1("", shared_secret, encoded_public_key);

// Query Mojang to verify client authenticated
let profile = GET https://sessionserver.mojang.com/session/minecraft/hasJoined
    ?username={username}
    &serverId={server_hash}
```

Mojang returns the player profile if authentication succeeded:

```json
{
  "id": "8667ba71b85a4004af54457a9734eed7",
  "name": "Steve",
  "properties": [
    {
      "name": "textures",
      "value": "<base64 skin/cape data>",
      "signature": "<cryptographic signature>"
    }
  ]
}
```

### Step 6: Enable Encryption

Both sides now enable AES-128-CFB8 encryption using the shared secret:

```rust
// Create cipher pair (uses same key for both directions)
let encryptor = Aes128Cfb8::new(shared_secret, shared_secret);
let decryptor = Aes128Cfb8::new(shared_secret, shared_secret);

// All further packets are encrypted
```

```
Passage → Player: Login Success {
    uuid: "8667ba71-b85a-4004-af54-457a9734eed7",
    username: "Steve",
    properties: [...]
}
```

Connection is now fully authenticated and encrypted.

## Cryptographic Details

### RSA Key Generation

Passage generates a 1024-bit RSA keypair at startup:

```rust
// Uses OS random number generator (SysRng)
let mut rng = SysRng;
let private_key = RsaPrivateKey::new(&mut rng, 1024)?;
let public_key = RsaPublicKey::from(&private_key);
```

The same keypair is used for all connections throughout Passage's lifetime.

### Minecraft Hash Function

Minecraft uses a special SHA-1 hash format with signed byte representation:

```rust
fn minecraft_hash(server_id: &str, shared_secret: &[u8], public_key: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(server_id);        // Always "" for online mode
    hasher.update(shared_secret);    // 16 bytes
    hasher.update(public_key);       // DER-encoded RSA public key

    // Convert to signed BigInt and represent in hex (Minecraft-specific)
    BigInt::from_signed_bytes_be(&hasher.finalize()).to_str_radix(16)
}
```

This produces hashes like `-7c9d5b0044c130109a5d7b5fb5c317c02b4e28c1`.

### AES-128-CFB8 Encryption

After authentication, all packets are encrypted with AES-128 in CFB8 mode:

- **Algorithm**: AES (Advanced Encryption Standard)
- **Key size**: 128 bits (16 bytes)
- **Mode**: CFB8 (Cipher Feedback, 8-bit)
- **Key**: The shared secret (same 16 bytes)
- **IV**: The shared secret (same 16 bytes) - unusual but per Minecraft protocol

```rust
// Both encryption and decryption use the same key and IV
let key = shared_secret;  // 16 bytes
let iv = shared_secret;   // 16 bytes (same!)

let encryptor = Aes128Cfb8Enc::new_from_slices(key, iv)?;
let decryptor = Aes128Cfb8Dec::new_from_slices(key, iv)?;
```

Every packet after Login Success is encrypted/decrypted byte-by-byte.

## Authentication Cookies

To speed up reconnections, Passage optionally uses HMAC-signed authentication cookies.

### Cookie Generation

When a player successfully authenticates, Passage can store authentication data in a cookie:

```rust
const AUTH_COOKIE_KEY: &str = "passage:authentication";
const AUTH_COOKIE_EXPIRY: Duration = 6 hours;

// Sign the profile data with HMAC-SHA256
fn sign_cookie(profile: &Profile, secret: &[u8]) -> Vec<u8> {
    let data = serialize(profile);  // Profile as bytes
    let mac = HmacSha256::new(secret);
    mac.update(&data);
    let signature = mac.finalize();

    // Cookie format: [32-byte signature][profile data]
    [signature, data].concat()
}
```

The signature prevents tampering - if the cookie is modified, the signature won't match.

### Cookie Verification

On reconnection, if the client presents a valid cookie:

```rust
fn verify_cookie(cookie: &[u8], secret: &[u8]) -> Option<Profile> {
    if cookie.len() < 32 {
        return None;
    }

    let (signature, data) = cookie.split_at(32);

    // Verify HMAC signature
    let mac = HmacSha256::new(secret);
    mac.update(data);
    if !mac.verify(signature).is_ok() {
        return None;
    }

    // Deserialize profile
    deserialize(data)
}
```

If valid, Passage skips Mojang authentication entirely, reducing connection time from ~500ms to ~50ms.

### Cookie Security

- **Signed with HMAC-SHA256**: Cannot be forged without the secret key
- **Secret key**: Configured in `auth_secret` (loaded from file or env var)
- **Expiry**: 6 hours by default
- **Transmitted**: In Minecraft's cookie storage packet (introduced in 1.20.5)
- **Scope**: Per-server (cookies aren't shared across different servers)

### Session Cookies

Passage also uses session cookies for tracking connection state:

```rust
const SESSION_COOKIE_KEY: &str = "passage:session";

// Stores temporary session data (if needed)
```

## Security Properties

### Why This is Secure

1. **Mojang Validation**: Only players with valid Mojang accounts can authenticate
2. **No Credential Exposure**: Passage never sees the player's password or access token
3. **Man-in-the-Middle Protection**: RSA + session server validation prevents MITM
4. **Encrypted Communication**: AES-128 protects all data after authentication
5. **Replay Protection**: Each connection uses a unique shared secret and verify token
6. **Cookie Integrity**: HMAC ensures cookies can't be forged or tampered with

### What Passage Can See

After authentication, Passage has access to:
- Player's UUID
- Player's username
- Player's skin/cape (texture properties)
- All encrypted packets (since Passage has the shared secret)

### What Passage Cannot See

- Player's Mojang account password
- Player's Mojang access token
- Player's email address
- Payment information

## Chat Signing Preservation

Unlike traditional proxies, Passage preserves Mojang's chat signing:

**Traditional Proxies:**
- Must decrypt, modify, and re-encrypt all packets
- Break the chat signing chain
- Cannot preserve cryptographic signatures

**Passage:**
- Only handles login/configuration phase
- Transfers player directly to backend server
- Backend receives the original, signed chat messages
- Full chat signing support out of the box

This means:
✅ Chat reporting works correctly
✅ Signed messages are cryptographically verifiable
✅ Server operators can trust message authenticity

## Offline Mode (Not Supported)

Passage does NOT support offline mode servers:

- All players must have valid Mojang accounts
- Authentication with Mojang's session servers is required
- This is a security feature, not a limitation

If you need offline mode, you should use a traditional proxy like Velocity or BungeeCord.

## Security Best Practices

### Auth Secret Management

```bash
# Generate a strong secret
openssl rand -base64 32 > config/auth_secret

# Secure file permissions
chmod 600 config/auth_secret

# Never commit to version control
echo "config/auth_secret" >> .gitignore
```

### In Kubernetes

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: passage-auth-secret
type: Opaque
data:
  auth_secret: <base64-encoded secret>
---
apiVersion: apps/v1
kind: Deployment
spec:
  template:
    spec:
      containers:
      - name: passage
        env:
        - name: AUTH_SECRET_FILE
          value: /run/secrets/auth-secret
        volumeMounts:
        - name: auth-secret
          mountPath: /run/secrets
          readOnly: true
      volumes:
      - name: auth-secret
        secret:
          secretName: passage-auth-secret
```

### Monitoring

Monitor authentication metrics:
- Authentication success/failure rate
- Mojang API latency
- Cookie hit rate (if using cookies)
- Encryption errors

## Troubleshooting

### "Invalid session" error

**Cause:** Client's session token is invalid or expired

**Solution:**
- Player needs to restart Minecraft client
- Check Mojang services are online
- Verify server is in online mode

### "Failed to verify username"

**Cause:** Mojang session servers rejected the authentication

**Possible reasons:**
- Player isn't logged into Minecraft
- Network issues reaching sessionserver.mojang.com
- Mojang services are down
- Player's account is suspended

**Solution:**
- Check Mojang service status
- Verify network connectivity to Mojang
- Player should re-authenticate in launcher

### Authentication takes too long

**Cause:** High latency to Mojang's session servers

**Solution:**
- Enable authentication cookies to skip Mojang on reconnects
- Check network latency to sessionserver.mojang.com
- Consider geographic factors (Mojang servers are centralized)

### Cookie authentication not working

**Causes:**
- `auth_secret` not configured or changed
- Cookie expired (>6 hours old)
- Cookie was tampered with

**Solution:**
- Verify `auth_secret` is configured consistently
- Check cookie expiry time
- Ensure secure transmission of cookies

## Performance Impact

Typical authentication timing:

| Phase | Duration | Notes |
|-------|----------|-------|
| RSA key exchange | <1ms | One-time at startup |
| Client calculates hash | ~10ms | Client-side |
| Mojang authentication | 100-500ms | Network latency |
| Server verification | 100-500ms | Network latency |
| Enable encryption | <1ms | Local operation |
| **Total (first time)** | **~200-1000ms** | **Mostly network** |
| **With cookie** | **~50ms** | **No Mojang API** |

The majority of time is spent waiting for Mojang's APIs. This is unavoidable for first-time connections.
