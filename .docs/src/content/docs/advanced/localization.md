---
title: Localization
description: Learn how to configure multi-language support in Passage.
---

This guide shows you how to configure Passage to display disconnect messages in multiple languages, providing a better experience for international players.

## Overview

Passage supports localization (l10n) for disconnect messages. When a player is disconnected, Passage will show the message in the player's configured language.

**Supported scenarios:**
- Connection timeouts
- No available backend servers
- Resource pack failures
- Custom disconnect reasons

## Configuration

Configure localization in `config.toml`:

```toml
[localization]
default_locale = "en_US"

[localization.messages.en]
disconnect_timeout = "{\"text\":\"Connection timeout\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"No server available\",\"color\":\"yellow\"}"

[localization.messages.es]
disconnect_timeout = "{\"text\":\"Tiempo de espera agotado\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"Servidor no disponible\",\"color\":\"yellow\"}"
```

---

## Locale Codes

Passage uses standard Minecraft locale codes:

### Supported Locales

| Code | Language | Region |
|------|----------|--------|
| `en_US` | English | United States |
| `en_GB` | English | United Kingdom |
| `de_DE` | German | Germany |
| `es_ES` | Spanish | Spain |
| `es_MX` | Spanish | Mexico |
| `fr_FR` | French | France |
| `fr_CA` | French | Canada |
| `it_IT` | Italian | Italy |
| `pt_BR` | Portuguese | Brazil |
| `pt_PT` | Portuguese | Portugal |
| `ru_RU` | Russian | Russia |
| `ja_JP` | Japanese | Japan |
| `ko_KR` | Korean | South Korea |
| `zh_CN` | Chinese | China (Simplified) |
| `zh_TW` | Chinese | Taiwan (Traditional) |
| `nl_NL` | Dutch | Netherlands |
| `pl_PL` | Polish | Poland |
| `tr_TR` | Turkish | Turkey |
| `sv_SE` | Swedish | Sweden |
| `da_DK` | Danish | Denmark |
| `fi_FI` | Finnish | Finland |
| `no_NO` | Norwegian | Norway |
| `cs_CZ` | Czech | Czech Republic |
| `el_GR` | Greek | Greece |
| `hu_HU` | Hungarian | Hungary |
| `ro_RO` | Romanian | Romania |
| `uk_UA` | Ukrainian | Ukraine |
| `th_TH` | Thai | Thailand |
| `vi_VN` | Vietnamese | Vietnam |
| `id_ID` | Indonesian | Indonesia |
| `ms_MY` | Malay | Malaysia |
| `ar_SA` | Arabic | Saudi Arabia |
| `he_IL` | Hebrew | Israel |

---

## Default Locale

The `default_locale` is used when:
- The client doesn't send a locale preference
- The client's locale is not configured in Passage
- Fallback is needed for any reason

```toml
[localization]
default_locale = "en_US"  # Used as fallback
```

**Best practice:** Always configure messages for your `default_locale`.

---

## Message Keys

### Standard Messages

Passage provides these built-in message keys:

#### `disconnect_timeout`

Shown when a connection times out during handshake.

**When it occurs:**
- Client doesn't respond within the configured `timeout` period
- Network issues cause connection to stall

**Example:**
```toml
[localization.messages.en]
disconnect_timeout = "{\"text\":\"Connection timeout\",\"color\":\"red\"}"
```

---

#### `disconnect_no_target`

Shown when no backend server is available.

**When it occurs:**
- Target discovery returns zero servers
- All servers are full (player_fill strategy)
- Strategy adapter returns no target

**Example:**
```toml
[localization.messages.en]
disconnect_no_target = "{\"text\":\"No server available. Please try again later.\",\"color\":\"yellow\"}"
```

---

#### `disconnect_failed_resourcepack`

Shown when a required resource pack fails to load.

**When it occurs:**
- Client rejects the resource pack
- Resource pack download fails
- Resource pack is corrupted

**Example:**
```toml
[localization.messages.en]
disconnect_failed_resourcepack = "{\"text\":\"Failed to load required resource pack\",\"color\":\"red\"}"
```

---

## Message Format

Messages use Minecraft's JSON text component format.

### Basic Text

```toml
disconnect_timeout = "{\"text\":\"Connection timeout\"}"
```

### Colored Text

```toml
disconnect_timeout = "{\"text\":\"Connection timeout\",\"color\":\"red\"}"
```

**Available colors:**
- `black`, `dark_blue`, `dark_green`, `dark_aqua`
- `dark_red`, `dark_purple`, `gold`, `gray`
- `dark_gray`, `blue`, `green`, `aqua`
- `red`, `light_purple`, `yellow`, `white`

### Formatted Text

```toml
disconnect_timeout = "{\"text\":\"Connection timeout\",\"bold\":true,\"color\":\"red\"}"
```

**Available formatting:**
- `"bold": true`
- `"italic": true`
- `"underlined": true`
- `"strikethrough": true`
- `"obfuscated": true`

### Complex Messages

```toml
disconnect_no_target = "{\"text\":\"\",\"extra\":[{\"text\":\"No server available\\n\",\"color\":\"yellow\",\"bold\":true},{\"text\":\"Please try again later\",\"color\":\"gray\"}]}"
```

### With Placeholders

Messages support parameter substitution:

```toml
disconnect_custom = "{\"text\":\"Hello {player}!\"}"
```

**Available placeholders:**
- `{player}` - Player username
- `{server}` - Server hostname
- `{reason}` - Disconnect reason (if provided)

**Note:** Standard messages (`disconnect_timeout`, `disconnect_no_target`, etc.) don't use placeholders by default.

---

## Example Configurations

### English Only

```toml
[localization]
default_locale = "en_US"

[localization.messages.en]
disconnect_timeout = "{\"text\":\"Connection timeout\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"No server available\",\"color\":\"yellow\"}"
disconnect_failed_resourcepack = "{\"text\":\"Failed to load resource pack\",\"color\":\"red\"}"
```

---

### Multi-Language (English + Spanish)

```toml
[localization]
default_locale = "en_US"

[localization.messages.en]
disconnect_timeout = "{\"text\":\"Connection timeout\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"No server available\",\"color\":\"yellow\"}"
disconnect_failed_resourcepack = "{\"text\":\"Failed to load resource pack\",\"color\":\"red\"}"

[localization.messages.es]
disconnect_timeout = "{\"text\":\"Tiempo de espera agotado\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"Servidor no disponible\",\"color\":\"yellow\"}"
disconnect_failed_resourcepack = "{\"text\":\"Error al cargar el paquete de recursos\",\"color\":\"red\"}"
```

---

### European Languages

```toml
[localization]
default_locale = "en_GB"

[localization.messages.en]
disconnect_timeout = "{\"text\":\"Connection timeout\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"No server available\",\"color\":\"yellow\"}"

[localization.messages.de]
disconnect_timeout = "{\"text\":\"Verbindungszeitüberschreitung\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"Kein Server verfügbar\",\"color\":\"yellow\"}"

[localization.messages.fr]
disconnect_timeout = "{\"text\":\"Délai de connexion dépassé\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"Aucun serveur disponible\",\"color\":\"yellow\"}"

[localization.messages.it]
disconnect_timeout = "{\"text\":\"Timeout di connessione\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"Nessun server disponibile\",\"color\":\"yellow\"}"

[localization.messages.es]
disconnect_timeout = "{\"text\":\"Tiempo de espera agotado\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"Servidor no disponible\",\"color\":\"yellow\"}"

[localization.messages.pt]
disconnect_timeout = "{\"text\":\"Tempo de conexão esgotado\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"Nenhum servidor disponível\",\"color\":\"yellow\"}"
```

---

### Asian Languages

```toml
[localization]
default_locale = "en_US"

[localization.messages.en]
disconnect_timeout = "{\"text\":\"Connection timeout\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"No server available\",\"color\":\"yellow\"}"

[localization.messages.ja]
disconnect_timeout = "{\"text\":\"接続タイムアウト\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"利用可能なサーバーがありません\",\"color\":\"yellow\"}"

[localization.messages.ko]
disconnect_timeout = "{\"text\":\"연결 시간 초과\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"사용 가능한 서버가 없습니다\",\"color\":\"yellow\"}"

[localization.messages.zh_CN]
disconnect_timeout = "{\"text\":\"连接超时\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"没有可用的服务器\",\"color\":\"yellow\"}"

[localization.messages.zh_TW]
disconnect_timeout = "{\"text\":\"連線逾時\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"沒有可用的伺服器\",\"color\":\"yellow\"}"
```

---

### Complete World-Wide Setup

```toml
[localization]
default_locale = "en_US"

# English
[localization.messages.en]
disconnect_timeout = "{\"text\":\"Connection timeout\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"No server available\",\"color\":\"yellow\"}"

# German
[localization.messages.de]
disconnect_timeout = "{\"text\":\"Verbindungszeitüberschreitung\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"Kein Server verfügbar\",\"color\":\"yellow\"}"

# Spanish
[localization.messages.es]
disconnect_timeout = "{\"text\":\"Tiempo de espera agotado\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"Servidor no disponible\",\"color\":\"yellow\"}"

# French
[localization.messages.fr]
disconnect_timeout = "{\"text\":\"Délai de connexion dépassé\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"Aucun serveur disponible\",\"color\":\"yellow\"}"

# Italian
[localization.messages.it]
disconnect_timeout = "{\"text\":\"Timeout di connessione\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"Nessun server disponibile\",\"color\":\"yellow\"}"

# Portuguese (Brazil)
[localization.messages.pt_BR]
disconnect_timeout = "{\"text\":\"Tempo de conexão esgotado\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"Nenhum servidor disponível\",\"color\":\"yellow\"}"

# Russian
[localization.messages.ru]
disconnect_timeout = "{\"text\":\"Тайм-аут подключения\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"Нет доступного сервера\",\"color\":\"yellow\"}"

# Japanese
[localization.messages.ja]
disconnect_timeout = "{\"text\":\"接続タイムアウト\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"利用可能なサーバーがありません\",\"color\":\"yellow\"}"

# Korean
[localization.messages.ko]
disconnect_timeout = "{\"text\":\"연결 시간 초과\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"사용 가능한 서버가 없습니다\",\"color\":\"yellow\"}"

# Chinese (Simplified)
[localization.messages.zh_CN]
disconnect_timeout = "{\"text\":\"连接超时\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"没有可用的服务器\",\"color\":\"yellow\"}"

# Dutch
[localization.messages.nl]
disconnect_timeout = "{\"text\":\"Verbindingstime-out\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"Geen server beschikbaar\",\"color\":\"yellow\"}"

# Polish
[localization.messages.pl]
disconnect_timeout = "{\"text\":\"Przekroczono limit czasu połączenia\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"Brak dostępnego serwera\",\"color\":\"yellow\"}"

# Turkish
[localization.messages.tr]
disconnect_timeout = "{\"text\":\"Bağlantı zaman aşımı\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"Kullanılabilir sunucu yok\",\"color\":\"yellow\"}"
```

---

## Environment Variables

Override localization settings with environment variables:

```bash
# Set default locale
export PASSAGE_LOCALIZATION_DEFAULT_LOCALE=de_DE

# Override specific messages (not commonly used)
export PASSAGE_LOCALIZATION_MESSAGES_EN_DISCONNECT_TIMEOUT='{"text":"Connection timeout","color":"red"}'
```

**Note:** Environment variable syntax for nested localization messages is cumbersome. Prefer using `config.toml`.

---

## How Locale Selection Works

When a player connects, Passage:

1. **Checks client locale** - Reads the locale sent by the client during handshake
2. **Looks up messages** - Searches for messages in `localization.messages.<locale>`
3. **Falls back** - If not found, uses `default_locale` messages
4. **Displays message** - Sends the localized message to the client

**Example flow:**
- Client with locale `es_MX` connects
- Connection times out
- Passage looks for `localization.messages.es_MX.disconnect_timeout`
- If not found, checks `localization.messages.es.disconnect_timeout` (language fallback)
- If not found, uses `localization.messages.en_US.disconnect_timeout` (default locale)

---

## Testing Localization

### Change Client Locale

In Minecraft:
1. **Options → Language**
2. Select a language
3. Reconnect to test

### Test Disconnect Messages

#### Test Timeout
```toml
# Set very short timeout
timeout = 1
```
Connect and wait 1 second without completing login.

#### Test No Target
```toml
[target_discovery]
adapter = "fixed"
# Don't configure any targets

[[target_discovery.fixed.targets]]
# Empty list
```

---

## Best Practices

### Coverage
- Always configure your `default_locale`
- Add locales for your primary player base
- Consider adding major languages (English, Spanish, German, French)

### Message Quality
- Keep messages clear and concise
- Use friendly, helpful tone
- Include actionable information when possible
- Test with native speakers

### Formatting
- Use colors to indicate severity:
  - Red for errors
  - Yellow for warnings
  - Gray/white for informational
- Keep formatting consistent across languages
- Avoid overly fancy formatting (readability first)

### Maintenance
- Keep messages up to date
- Review translations periodically
- Accept community contributions
- Store translations in version control

---

## Translation Services

For professional translations, consider:

- **Crowdin** - Community translation platform
- **POEditor** - Translation management
- **DeepL** - AI translation (review required)
- **Google Translate** - Quick drafts (review required)
- **Native speakers** - Best quality

---

## Common Translations

### "Connection timeout"

| Language | Translation |
|----------|-------------|
| German | Verbindungszeitüberschreitung |
| Spanish | Tiempo de espera agotado |
| French | Délai de connexion dépassé |
| Italian | Timeout di connessione |
| Portuguese | Tempo de conexão esgotado |
| Russian | Тайм-аут подключения |
| Japanese | 接続タイムアウト |
| Korean | 연결 시간 초과 |
| Chinese | 连接超时 |
| Dutch | Verbindingstime-out |
| Polish | Przekroczono limit czasu |
| Turkish | Bağlantı zaman aşımı |

### "No server available"

| Language | Translation |
|----------|-------------|
| German | Kein Server verfügbar |
| Spanish | Servidor no disponible |
| French | Aucun serveur disponible |
| Italian | Nessun server disponibile |
| Portuguese | Nenhum servidor disponível |
| Russian | Нет доступного сервера |
| Japanese | 利用可能なサーバーがありません |
| Korean | 사용 가능한 서버가 없습니다 |
| Chinese | 没有可用的服务器 |
| Dutch | Geen server beschikbaar |
| Polish | Brak dostępnego serwera |
| Turkish | Kullanılabilir sunucu yok |

---

## Troubleshooting

### Messages Not Appearing in Different Languages

1. **Check locale code spelling:**
   ```toml
   [localization.messages.es_ES]  # Correct
   [localization.messages.es-ES]  # Wrong - use underscore
   ```

2. **Verify JSON format:**
   ```bash
   # Test JSON validity
   echo '{"text":"Hello"}' | jq
   ```

3. **Check client language settings** in Minecraft

4. **Enable debug logging:**
   ```bash
   RUST_LOG=debug passage
   ```

### Special Characters Not Displaying

Ensure proper escaping in TOML:

```toml
# Escape quotes
disconnect_timeout = "{\"text\":\"Connection timeout\"}"

# Escape backslashes
disconnect_timeout = "{\"text\":\"Error: \\\\\"}"

# Use multiline strings for complex JSON
disconnect_timeout = '''{"text":"Connection timeout","color":"red"}'''
```
