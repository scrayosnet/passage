---
title: Cookies
description: Learn what cookies are available and how to use them.
---

Passage uses Minecraft's cookie storage mechanism (introduced in 1.20.5) to cache authentication data and track sessions. Two cookies are used:

- **`passage:authentication`** - HMAC-signed cookie that enables fast re-authentication by skipping Mojang API calls
- **`passage:session`** - Session tracking with unique UUIDs across reconnections

## Authentication Cookie

Key: `passage:authentication`

Caches validated player profile data to skip Mojang API calls on reconnection. Binary format: 32-byte HMAC-SHA256 signature followed by JSON data.

### Contents

```json
{
  "timestamp": 1707542400,
  "client_addr": "192.168.1.100:54321",
  "user_name": "Steve",
  "user_id": "8667ba71-b85a-4004-af54-457a9734eed7",
  "target": "lobby-01",
  "profile_properties": [
    {
      "name": "textures",
      "value": "<base64 skin/cape data>",
      "signature": "<cryptographic signature>"
    }
  ],
  "extra": {}
}
```

- **timestamp**: Unix timestamp when created (updated on every transfer)
- **client_addr**: IP/port binding for security
- **user_name/user_id**: Player identity
- **target**: Last connected backend server (updated on every transfer)
- **profile_properties**: Mojang skin/cape data
- **extra**: Custom system-specific data

**Note**: Only `timestamp` and `target` are updated on each transfer. All other fields remain from the initial authentication.

### Security & Lifecycle

**Security:**
- HMAC-SHA256 signature prevents tampering/forgery
- IP address binding (only valid from same IP)
- Configurable expiry (default 6 hours, recommended: 1 minute for production)
- Requires `auth_secret` configuration (must be shared across all Passage and backend servers)

**Usage Flow:**
1. Player authenticates with Mojang (first connection)
2. Passage signs and stores cookie to client with current timestamp and target
3. On every transfer/reconnection, client returns cookie
4. Passage or backend server verifies signature, expiry, and IP
5. Valid → skip Mojang API and update cookie (timestamp + target only); Invalid → full authentication
6. Cookie is refreshed on every transfer, extending the expiry window for active players

**Important**: The authentication cookie is updated on **every transfer** with a fresh timestamp and current target. This extends the expiry window for active players, ensuring they remain authenticated across server switches without re-contacting Mojang. Only the `timestamp` and `target` fields are updated; all other authentication data (username, UUID, profile properties) remains from the initial Mojang authentication.

### Configuration

```toml
auth_secret_file = "config/auth_secret"
auth_cookie_expiry_secs = 60  # Recommended: 60 seconds (1 minute)
```

The `auth_cookie_expiry_secs` setting controls how long cookies remain valid. **Recommended: 60 seconds (1 minute)** for production environments. Since cookies are refreshed on every transfer, active players will never experience authentication issues. Only returning players who haven't connected within the expiry window will need to re-authenticate with Mojang.

Generate and secure the secret:

```bash
openssl rand -base64 32 > config/auth_secret
chmod 600 config/auth_secret
echo "config/auth_secret" >> .gitignore
```

## Session Cookie

Key: `passage:session`

Tracks unique player sessions across reconnections using a UUIDv4. Not signed (no sensitive data). Persists until client restart.

### Contents

```json
{
  "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "server_address": "play.example.com",
  "server_port": 25565
}
```

- **id**: Unique session identifier (UUIDv4)
- **server_address/server_port**: From handshake

### Use Cases

- Track unique sessions and identify reconnections
- Correlate logs/traces across connections
- Analytics for connection patterns and server preferences
- Foundation for future session-based features

## Backend Server Integration

Backend servers **should** use the same authentication cookie mechanism to avoid redundant Mojang authentication:

1. **Share the `auth_secret`**: All Passage instances and backend servers must use the same secret
2. **Validate cookies**: Backend servers can verify the cookie signature and expiry using the shared secret
3. **Skip authentication**: When a valid cookie is present, backend servers can trust the player identity without contacting Mojang
4. **Update cookies**: Backend servers should refresh the cookie timestamp on each transfer to extend validity

This creates a **chain of trust** across your entire server network:
- Player authenticates with Mojang once (on first connection)
- Cookie is validated and refreshed at each hop (Passage → Backend Server A → Backend Server B)
- No Mojang API calls needed for transfers between backend servers
- Active players never need to re-authenticate with Mojang (cookie refreshed on every transfer)
- Returning players re-authenticate if they haven't connected within the expiry window

## Performance Benefits

Authentication cookies eliminate Mojang API calls for reconnections and server transfers:

- **Reduced Mojang API dependency**: Active players don't trigger Mojang authentication requests
- **Faster transfers**: Skip authentication handshake between server switches
- **Improved reliability**: Service remains functional even if Mojang APIs are slow or unavailable
- **Lower latency**: Local cookie validation is faster than remote API calls

## Security Best Practices

Generate and protect the auth secret. **This secret must be identical** across all Passage instances and backend servers:

```bash
openssl rand -base64 32 > config/auth_secret
chmod 600 config/auth_secret
echo "config/auth_secret" >> .gitignore
```

For Kubernetes, use a Secret and mount it on all servers:

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: passage-auth-secret
stringData:
  auth_secret: "your-base64-secret-here"
---
# Mount in ALL Passage and backend server deployments
env:
- name: AUTH_SECRET_FILE
  value: /run/secrets/auth-secret
volumeMounts:
- name: auth-secret
  mountPath: /run/secrets
  readOnly: true
```

**Secret sharing**: All servers in your network (Passage instances and backend Minecraft servers) must share the same secret to validate cookies.

**Secret rotation**: Rotating invalidates all existing auth cookies. Players will need to re-authenticate with Mojang on their next connection.

## Troubleshooting

**Auth cookie not working** (always full Mojang auth):
- Check `auth_secret` is configured and consistent across all servers
- Cookie expires after configured duration (default 6 hours, recommended 60 seconds)
- IP address must match (NAT changes invalidate cookie)
- Verify `auth_cookie_expiry_secs` is configured appropriately
- Check logs for validation errors

**Session cookie not persisting** (new ID every time):
- Expected behavior on client restart
- Cookie clears when Minecraft closes
- Ensure handshake `server_address` matches

**Cookie sizes**: Auth ~500-1500 bytes, Session ~100-200 bytes
