# Relay mobile

The phone half of Relay, Relay first. The app opens on the PC: the agents running in the
Relay engine on your desktop, reached over your own WiFi when you are home and through
Tailscale or a server you host when you are not. Below the PC in the drawer is what runs on
the phone itself: chats with a model that lives there or an API you choose. One app, one look,
one drawer.

Everything about the PC link — pairing, routes, what the tab can do, and the security model —
is in [docs/MOBILE.md](../../docs/MOBILE.md) at the repository root. The desktop client is
`apps/relay-native`; the engine is `crates/`; the door the phone talks to is
`crates/relay-remote`.

## Where things are

| path | what |
| --- | --- |
| `app/screens/RelayScreen/` | the home screen (the PC): sessions, terminal, board, changes, paired PCs, pairing |
| `app/components/views/SettingsDrawer/` | the drawer: the PC first, then Local (characters, models, recent chats) |
| `lib/engine/Relay/` | the phone's end of the bus: the WebSocket client, the terminal screen, attention, on-device summary |
| `lib/state/RelayHosts.ts` | the PCs this phone paired with |
| `app/screens/ChatScreen/`, `lib/engine/` | chats, on device and over an API |
| `i18n/` | the UI strings (English only); the PC screens use literal strings |

## Run

An Expo / React Native app. The **Mobile APK** workflow (Actions tab, or a `mobile-v*` tag)
builds an installable `relay-mobile-apk`. Locally, with a Java 17/21 SDK and the Android SDK:

```fish
npm ci
npx expo run:android
```

`eas.json.example` is the EAS profile for a local build (`eas build --platform android --local`).
iOS is not built.

## Verify

```fish
npm ci && npx tsc --noEmit -p tsconfig.json && npm run lint
```

Both run in CI. The bus payloads the phone sends are exercised against a real engine by
`cargo test -p relay-remote`.

## Privacy

- Chats never use the PC link. On device, the model runs on the phone; the fallback to an
  API (Settings → Privacy) only applies when no on-device model can run, tells you each time,
  and can be switched off.
- The PC link carries only what you do on the PC screens, and every action is logged on the
  desktop as yours.
- The pairing credential stays on the phone. Each connection proves it with a one-time
  challenge instead of sending it.

## Models and backends

On device, chats run GGUF models through [llama.cpp](https://github.com/ggml-org/llama.cpp).
Over an API, the app speaks to koboldcpp, Ollama, text-generation-webui, OpenAI, Claude,
Cohere, OpenRouter and any text- or chat-completion backend through its template system
([docs/CustomTemplates.md](docs/CustomTemplates.md)).

## Licence

[AGPL-3.0](LICENSE). This app is a modified version of an AGPL-3.0 program; the licence
requires this notice and the licence text to stay with the source.
