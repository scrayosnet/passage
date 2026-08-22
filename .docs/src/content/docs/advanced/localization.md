---
title: Localization
description: Configure multi-language disconnect messages in Passage.
sidebar:
    order: 4
---

Passage supports localization for disconnect messages. When a player is disconnected, Passage displays the message in the player's client language.

## Configuration

Localization is a **per-route adapter** configured in `routes[].localization`:

```yaml
routes:
- hostname: "mc.example.net"
  localization:
    type: fixed
    default_locale: "en_US"
    warn_unknown_keys: true
    messages:
      en:
        locale: "English"
        disconnect_timeout: '{"text":"Connection timed out","color":"red"}'
        disconnect_no_target: '{"text":"No server available","color":"yellow"}'
        disconnect_unauthenticated: '{"text":"Authentication failed","color":"red"}'
        disconnect_unsupported: '{"text":"Please use {preferred}","color":"red"}'
      de:
        locale: "Deutsch"
        disconnect_timeout: '{"text":"Verbindung getrennt: Keine Antwort vom Client","color":"red"}'
        disconnect_no_target: '{"text":"Kein verfügbarer Server","color":"yellow"}'
        disconnect_unauthenticated: '{"text":"Authentifizierung fehlgeschlagen","color":"red"}'
        disconnect_unsupported: '{"text":"Bitte nutze {preferred}","color":"red"}'
```

### Adapter Types

| Type | Description |
|------|-------------|
| `fixed` (default) | Returns messages from a static configuration map |
| `grpc` | Delegates to an external gRPC service |

### Fixed Adapter Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `default_locale` | string | `"en_US"` | Fallback locale when the client's locale has no messages |
| `warn_unknown_keys` | bool | `true` | Log warnings for unrecognized message keys |
| `messages` | map | *(built-in defaults)* | Locale-keyed map of message key-value pairs |

### gRPC Adapter

For dynamic localization logic, delegate to a gRPC service:

```yaml
localization:
  type: grpc
  address: "http://localization-service:50051"
```

See the [gRPC Protocol Reference](/reference/grpc-protocol/#localization-service-localizationproto) for the service definition.

---

## Built-in Defaults

If you don't configure localization, Passage provides default messages in six languages:

| Locale Key | Language |
|------------|----------|
| `en` | English |
| `es` | Spanish |
| `fr` | French |
| `de` | German |
| `zh-CN` | Chinese (Simplified) |
| `ru` | Russian |

:::note[Locale Key Format]
Passage uses **short locale keys** like `en`, `de`, `es` -- not full codes like `en_US` or `de_DE`. The exception is regional variants like `zh-CN`.
:::

---

## Message Keys

Passage uses four built-in message keys:

### `disconnect_timeout`

Shown when the connection times out during handshake (keep-alive timeout).

### `disconnect_no_target`

Shown when no backend server is available -- either discovery returns zero targets, or all targets are filtered out by the actions pipeline.

### `disconnect_unauthenticated`

Shown when player authentication fails.

### `disconnect_unsupported`

Shown when the client speaks a protocol version below the minimum Passage supports (Minecraft 1.20.5, protocol `766`), which is the version that introduced the configuration phase Passage depends on.

| Parameter | Value |
|-----------|-------|
| `preferred` | The version name reported by the route's status adapter -- the same string the client sees in its server list |

:::note[Default Locale Only]
The client only reports its locale during the configuration phase, which these clients never reach. The message is therefore always resolved in the `default_locale`.
:::

### Custom Keys

gRPC adapters (Authentication and DiscoveryAction) can return custom localization keys to reject connections. These keys are resolved through the localization adapter:

```yaml
messages:
  en:
    disconnect_queue_full: '{"text":"Queue is full. Please try again later.","color":"yellow"}'
    disconnect_banned: '{"text":"You are banned from this server.","color":"red"}'
```

---

## Message Format

Messages use Minecraft's **JSON text component** format:

### Basic Text

```yaml
disconnect_timeout: '{"text":"Connection timeout"}'
```

### Colored Text

```yaml
disconnect_timeout: '{"text":"Connection timeout","color":"red"}'
```

Available colors: `black`, `dark_blue`, `dark_green`, `dark_aqua`, `dark_red`, `dark_purple`, `gold`, `gray`, `dark_gray`, `blue`, `green`, `aqua`, `red`, `light_purple`, `yellow`, `white`

### Formatted Text

```yaml
disconnect_timeout: '{"text":"Connection timeout","bold":true,"color":"red"}'
```

Formatting options: `bold`, `italic`, `underlined`, `strikethrough`, `obfuscated`

### Multi-line Messages

```yaml
disconnect_no_target: '{"text":"","extra":[{"text":"No server available\n","color":"yellow","bold":true},{"text":"Please try again later","color":"gray"}]}'
```

### Parameters

Some message keys are resolved with parameters. A parameter is substituted wherever its name appears in curly braces, so `{preferred}` is replaced by the value of the `preferred` parameter:

```yaml
disconnect_unsupported: '{"text":"Outdated client, please use {preferred}","color":"red"}'
```

Placeholders without a matching parameter are left untouched, and parameters that the message does not use are ignored. gRPC localization adapters receive the parameters as a name-value map and may format them however they like.

---

## Example: Multi-Language Setup

```yaml
routes:
- hostname: "mc.example.net"
  localization:
    type: fixed
    default_locale: "en_US"
    messages:
      en:
        locale: "English"
        disconnect_timeout: '{"text":"Disconnected: Connection timed out","color":"red"}'
        disconnect_no_target: '{"text":"Disconnected: No server available","color":"yellow"}'
        disconnect_unauthenticated: '{"text":"Disconnected: Authentication failed","color":"red"}'
        disconnect_unsupported: '{"text":"Disconnected: Outdated client, please use {preferred}","color":"red"}'
      es:
        locale: "Español"
        disconnect_timeout: '{"text":"Desconectado: Tiempo de espera agotado","color":"red"}'
        disconnect_no_target: '{"text":"Desconectado: No hay servidor disponible","color":"yellow"}'
        disconnect_unauthenticated: '{"text":"Desconectado: No se pudo autenticar","color":"red"}'
        disconnect_unsupported: '{"text":"Desconectado: Cliente obsoleto, usa {preferred}","color":"red"}'
      fr:
        locale: "Français"
        disconnect_timeout: '{"text":"Déconnecté : Délai de connexion dépassé","color":"red"}'
        disconnect_no_target: '{"text":"Déconnecté : Aucun serveur disponible","color":"yellow"}'
        disconnect_unauthenticated: '{"text":"Déconnecté : Échec de l''authentification","color":"red"}'
        disconnect_unsupported: '{"text":"Déconnecté : Client obsolète, veuillez utiliser {preferred}","color":"red"}'
      de:
        locale: "Deutsch"
        disconnect_timeout: '{"text":"Getrennt: Verbindungszeitüberschreitung","color":"red"}'
        disconnect_no_target: '{"text":"Getrennt: Kein Server verfügbar","color":"yellow"}'
        disconnect_unauthenticated: '{"text":"Getrennt: Authentifizierung fehlgeschlagen","color":"red"}'
        disconnect_unsupported: '{"text":"Getrennt: Veralteter Client, bitte nutze {preferred}","color":"red"}'
```

---

## How Locale Selection Works

1. The Minecraft client sends its configured language to Passage
2. Passage looks up messages matching the client's locale key (e.g., `de`)
3. If no match, falls back to the `default_locale` messages
4. If that also has no match, returns the raw key string

**To test:** Change your Minecraft language in **Options > Language**, then reconnect.

---

## Best Practices

- Always configure messages for your `default_locale` (usually `en`)
- Add locales for your primary player base's languages
- Keep messages clear, concise, and actionable
- Use colors consistently: red for errors, yellow for warnings
- Test messages with native speakers where possible
