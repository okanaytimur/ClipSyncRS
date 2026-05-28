# ClipSync — Rust + WebSocket Brifingi

Bu doküman, mevcut C# / SFTP tabanlı **ClipSync** uygulamasının Rust portunu sıfırdan
yazacak geliştiriciye (veya yeni bir Claude Code oturumuna) verilmek üzere yazıldı.
Hedef: iki makine arasında **kendi VPS'imizde koşan ufak bir relay sunucusu** üzerinden
pano (clipboard) metni senkronize etmek; istemci tarafında **2–3 MB tek dosya native
exe**, sunucu tarafında **~4–5 MB Linux binary'si**. Çalışma zamanı yok, Visual Studio
C++ workload'u gerekmez.

## Bağlam

- Referans uygulama: `C:\Users\okan\Desktop\netterm\` altında, GitHub'da `ClipSync`
  adıyla public. İki makine arasında **bir SFTP sunucusu üzerinden** pano metni
  senkronize ediyordu (`PollIntervalSeconds` ile her 4 saniyede bir
  `clipboard.json` indirip yükleyerek).
- Bu sürümde transport tamamen değişiyor: **SFTP → WebSocket relay**. 4 saniyelik
  polling gidiyor, push tabanlı oluyor. Sunucu kendi VPS'imizde koşacak.
- C# sürümüyle **tel protokolü uyumluluğu beklentisi yok** (transport farklı). C#
  kaynağı yalnızca *sync mantığı* ve *sonsuz döngü engeli* için referans.
- İkincil amaç: `russh` / OpenSSL / libssh2 zincirinden kurtulup saf Rust deps ile
  daha küçük binary üretmek.

## Mimari

```
[PC-A clipsync.exe] ──wss──┐
                           ├──► [VPS: clipsync-server] (relay, oda başına in-memory son mesaj cache)
[PC-B clipsync.exe] ──wss──┘
```

- İki istemci tek bir **oda**ya (`room_id`) kalıcı bir WebSocket bağlantısıyla
  bağlanır.
- Biri pano değişikliği gönderdiğinde sunucu odadaki **diğer** istemcilere anında
  broadcast eder.
- Sunucu her oda için **son mesajı** in-memory tutar; yeni bağlanan / yeniden bağlanan
  istemci connect anında bu son mesajı alır (bir tarafın kapalıyken yapılan kopya
  kaybolmasın diye).
- TLS sunucuda yapılmaz; önüne **Caddy** konur (1 satırlık reverse proxy +
  otomatik Let's Encrypt).

## Tel protokolü (WebSocket üzerinde JSON)

WS endpoint: `wss://<host>/ws?room=<room_id>&token=<token>`

Yanlış token → HTTP 401, kısa text body, upgrade reddedilir. Doğru token → upgrade
başarılı, istemci odaya katılır.

Mesaj zarfı (her iki yön):

```json
// pano içeriği
{ "type": "clip", "text": "...", "source_machine": "PC-NAME", "timestamp": "2026-05-28T12:00:00Z" }

// keepalive
{ "type": "ping" }
{ "type": "pong" }
```

- `timestamp` ISO-8601 UTC, ayraç `T`, sonek `Z`.
- `text` UTF-8 düz metin. **Maks. 1 MiB** — büyükse sunucu mesajı drop edip log'a yazar.
- Bilinmeyen `type` alanlı mesajlar sessizce yok sayılır (ileri uyumluluk).
- Her iki taraf 30 sn'de bir `ping` yollar, karşı taraf `pong` ile cevaplar. 60 sn
  içinde pong gelmezse bağlantı kopuk sayılır → reconnect.

## Sunucu (`clipsync-server`)

### Sorumluluklar

1. WS upgrade → token doğrulama → odaya ekle.
2. Yeni katılan istemciye, oda için cache'lenmiş `last` mesaj varsa onu yolla.
3. Bir istemciden gelen `clip` mesajını odadaki **diğer** istemcilere broadcast et;
   aynı zamanda `last`'i güncelle.
4. Ping/pong keepalive.
5. Loglama: stdout'a `[timestamp level] mesaj` satırları (systemd journal yutsun).

### State

```rust
struct AppState {
    rooms: Arc<DashMap<RoomId, RoomState>>,
    auth: Arc<HashMap<RoomId, Token>>, // config'ten yüklenir, immutable
}

struct RoomState {
    clients: Vec<ClientHandle>, // (client_id, mpsc::Sender<ServerMsg>)
    last: Option<ClipMessage>,
}
```

> `DashMap` kullanmak istemiyorsan `tokio::sync::RwLock<HashMap<...>>` da olur. Tek
> yazıcı yok ama oda sayısı düşük olacağı için lock contention ihmal edilebilir.

### Config (`config.toml`)

```toml
bind = "127.0.0.1:8080"   # Caddy önde, lokalde dinle

[[rooms]]
id = "okan-home"
token = "uzun-rastgele-string"     # openssl rand -hex 32

# [[rooms]]
# id = "ekip-x"
# token = "..."
```

Birden fazla oda destekli (bedava feature) ama tipik kullanım tek oda.

### Caddy reverse proxy örneği

```caddyfile
clip.ornek.com {
    reverse_proxy 127.0.0.1:8080
}
```

Caddy WS upgrade'i otomatik proxy'ler, ekstra ayar gerekmez.

### systemd unit (`clipsync-server.service`)

```ini
[Unit]
Description=ClipSync WebSocket relay
After=network.target

[Service]
Type=simple
ExecStart=/opt/clipsync/clipsync-server /opt/clipsync/config.toml
Restart=on-failure
RestartSec=2
User=clipsync
WorkingDirectory=/opt/clipsync

[Install]
WantedBy=multi-user.target
```

## İstemci (`clipsync` — Windows tray app)

### Davranış spesifikasyonu

1. **Açılış**: mevcut panoyu oku → `last_local_text`'e ata (kendi panomuzu boş yere
   yüklememek için). Sunucuya WS bağlantısı kur.
2. **Connect anında** sunucudan `clip` mesajı gelirse (cache'lenmiş son state):
   `source_machine != bu_makine` **ve** `text != last_local_text` ise panoya yaz +
   `last_local_text ← text`.
3. **Yerel pano değişti** (`WM_CLIPBOARDUPDATE`): panoyu oku → `last_local_text`'ten
   farklıysa `clip` mesajı olarak WS'ten yolla. `last_local_text`'i güncel pano
   metnine ata. Bağlantı kopuksa **pending** olarak tut, reconnect olunca yolla
   (sadece son pending — kuyruk değil, tek slot).
4. **Sunucudan `clip` mesajı geldi**: aynı kural — `source_machine != bu_makine` ve
   `text != last_local_text` ise panoya yaz + `last_local_text ← text`. **Bu adım
   kritik**: panoya yazdığımız metnin tekrar yüklenmesini engeller (sonsuz döngü
   engeli).

> Sonsuz döngü engelinin uzun açıklaması için C# `TrayApp.cs:288-303` referans.

### Reconnect mantığı

- Exponential backoff: 1s → 2s → 4s → 8s → 16s → 30s (cap).
- Başarılı bağlantıda backoff sıfırlanır.
- Bağlantı yokken UI thread'i etkilenmez; uygulama çalışmaya devam eder, tepsi durum
  ekranında "Bağlı değil — yeniden deneniyor" görünür.

### `config.json` (exe ile aynı klasörde, yoksa şablon olarak oluşturulur)

```json
{
  "ServerUrl": "wss://clip.ornek.com/ws",
  "RoomId": "okan-home",
  "Token": "uzun-rastgele-string"
}
```

PascalCase alan adları **bilinçli** (`#[serde(rename_all = "PascalCase")]`); kullanıcı
elle açıp yazacak.

### Tepsi menüsü

```
Durum Bilgisi
Şimdi Yeniden Bağlan
---
Çıkış
```

Tepsi ikonuna çift sol tık = Durum Bilgisi. Standart Win32 `MessageBoxW`:

```
Makine          : <hostname>
Sunucu          : <server-url>
Oda             : <room_id>
Bağlantı        : Bağlı | Bağlanıyor | Kopuk
Son başarı      : <YYYY-MM-DD HH:MM:SS veya ->
Son işlem       : <serbest metin>
```

### Loglama

- `clipsync.log` exe'nin yanında, UTF-8, append.
- Satır biçimi: `[2026-05-28 12:34:56] mesaj`.
- Hatalar yutulur ama log'a yazılır; uygulama asla çökmez.

## Mimari kararlar (kesin, sapma)

### Ortak

| Konu | Tercih | Gerekçe |
|---|---|---|
| Build profili | `cargo build --release` + `strip`, `lto=fat`, `codegen-units=1`, `panic="abort"` | Boyut hedefi için zorunlu |
| Async runtime | `tokio` | WS ekosistemi gereği |
| JSON | `serde` + `serde_json` (`#[serde(tag = "type")]` enum) | Standart |
| Zaman | `chrono` (RFC 3339 UTC) | Standart |
| Hata | `anyhow` (üst seviye) + `thiserror` (modül içi, opsiyonel) | Sade |

### Sunucu

| Konu | Tercih | Gerekçe |
|---|---|---|
| HTTP/WS framework | **`axum`** | `axum::extract::ws` resmi WS desteği, tokio-tungstenite üstüne |
| TLS | **Yok — Caddy önde** | Sertifika yönetimi Caddy'e, server düz HTTP |
| Map | `dashmap` veya `tokio::sync::RwLock<HashMap>` | Oda sayısı az, ikisi de uyar |
| Config | `toml` crate | İnsan-okur, küçük |
| Log | `tracing` + `tracing-subscriber` (sadece stdout, JSON gerekmez) | systemd journal yutar |

### İstemci

| Konu | Tercih | Gerekçe |
|---|---|---|
| Event loop | `tao` | `tray-icon` ile aynı ekip, HWND açar (WM_CLIPBOARDUPDATE için lazım) |
| Tepsi UI | `tray-icon` | Standart Rust seçimi |
| Pano oku/yaz | `clipboard-win` | Windows'a özel, owner kontrolü temiz |
| Pano değişim dinleme | `windows` crate ile `AddClipboardFormatListener` doğrudan | tao penceresini yeniden kullanmak için |
| WebSocket | **`tokio-tungstenite`** + `rustls` (`webpki-roots`) | Saf Rust, OpenSSL yok |
| Win32 P/Invoke | `windows` crate | Resmi MS bindings |
| Log | düz `std::fs::OpenOptions::append` + format makrosu | `tracing` istemcide fazla |

> Bunlar dışında bir crate eklemeden önce maliyeti (binary boyutuna katkı, derleme
> süresi) düşün.

## Thread modeli (istemci)

- **Ana thread (UI)**: `tao` event loop. WM_CLIPBOARDUPDATE, tray-icon olayları, menü
  komutları, **pano yazımı** burada (Windows clipboard owner thread'e bağlı).
- **Arka plan thread**: kendi tokio current-thread runtime'ı. WS bağlantısı, send/recv
  döngüsü, reconnect mantığı, ping/pong.
- İletişim: `tokio::sync::mpsc` çift yönlü kanallar.
  - UI → arka plan: `OutgoingClip(text)`, `Reconnect`, `Shutdown`.
  - Arka plan → UI: `IncomingClip(ClipMessage)`, `StatusChanged(ConnState)`. UI
    thread'inde almak için tao'nun `EventLoopProxy::send_event` mekanizması.

## Dosya düzeni (Cargo workspace)

```
clipsync/
├── Cargo.toml                       # [workspace]
├── Cargo.lock                       # commit edilir
├── README.md
├── .gitignore                       # /target, *.log, /config.json, /config.toml
├── client/
│   ├── Cargo.toml
│   ├── config.json.example
│   └── src/
│       ├── main.rs                  # giriş, tek-örnek mutex, tao event loop kurulumu
│       ├── tray.rs                  # tray-icon + menü + status MessageBox
│       ├── clipboard.rs             # clipboard-win + AddClipboardFormatListener
│       ├── ws.rs                    # tokio-tungstenite, connect/send/recv, reconnect, ping
│       ├── sync.rs                  # SyncEngine: kuyruk, last_local_text, sonsuz döngü engeli
│       ├── config.rs                # AppConfig okuma + ilk-açılış şablonu
│       └── log.rs                   # dosya logger
└── server/
    ├── Cargo.toml
    ├── config.toml.example
    ├── clipsync-server.service      # systemd unit
    └── src/
        ├── main.rs                  # axum router, config yükleme, graceful shutdown
        ├── ws.rs                    # /ws handler, upgrade, auth, oda yönetimi
        ├── room.rs                  # RoomState, broadcast, last cache
        └── config.rs                # ServerConfig (toml)
```

## `Cargo.toml` iskeletleri

### Workspace kök

```toml
[workspace]
members = ["client", "server"]
resolver = "2"

[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort"
strip = "symbols"
```

### `client/Cargo.toml`

```toml
[package]
name = "clipsync"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "clipsync"
path = "src/main.rs"

[dependencies]
tao             = "0.30"
tray-icon       = "0.19"
clipboard-win   = "5"
windows = { version = "0.58", features = [
    "Win32_Foundation",
    "Win32_System_DataExchange",
    "Win32_System_LibraryLoader",
    "Win32_UI_WindowsAndMessaging",
    "Win32_UI_Shell",
] }
tokio              = { version = "1", features = ["rt", "macros", "io-util", "sync", "time", "net"] }
tokio-tungstenite  = { version = "0.24", features = ["rustls-tls-webpki-roots"] }
rustls             = "0.23"
url                = "2"
serde              = { version = "1", features = ["derive"] }
serde_json         = "1"
chrono             = { version = "0.4", default-features = false, features = ["clock", "std", "serde"] }
anyhow             = "1"
```

### `server/Cargo.toml`

```toml
[package]
name = "clipsync-server"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "clipsync-server"
path = "src/main.rs"

[dependencies]
axum                = { version = "0.7", features = ["ws"] }
tokio               = { version = "1", features = ["rt-multi-thread", "macros", "signal", "sync", "time"] }
tokio-tungstenite   = "0.24"
serde               = { version = "1", features = ["derive"] }
serde_json          = "1"
toml                = "0.8"
chrono              = { version = "0.4", default-features = false, features = ["clock", "std", "serde"] }
anyhow              = "1"
tracing             = "0.1"
tracing-subscriber  = { version = "0.3", features = ["env-filter"] }
dashmap             = "6"
```

> Sürümler bu brifing yazıldığı andaki en güncel sürümler; proje başlangıcında
> `cargo search <crate>` ile doğrula.

## Kabul kriterleri

1. `cargo build --release --bin clipsync` çıktısı **≤ 4 MB** (hedef 2–3 MB).
2. `cargo build --release --bin clipsync-server` çıktısı **≤ 6 MB** Linux'ta.
3. İstemci tarafında sistem DLL'leri dışında runtime bağımlılığı yok
   (`dumpbin /dependents`).
4. exe başka bir Windows makinesine kopyalanıp çift tıklayınca, kullanıcının Rust
   SDK kurması gerekmeden çalışmalı.
5. İki makine aynı odada `clipsync.exe` ile çalışırken: bir tarafta kopyala →
   100 ms içinde diğer tarafın panosunda olmalı.
6. Sunucu yeniden başlasa istemciler exponential backoff ile yeniden bağlanmalı,
   uygulamalar çökmemeli.
7. Sonsuz döngü oluşmamalı (hızlı arka arkaya aynı metni kopyalama, iki makine
   aynı anda farklı şeyler kopyalama testleri).
8. `cargo clippy --release --workspace -- -D warnings` temiz.
9. Sunucu logu yanlış token denemesini, mesaj boyut aşımını, bağlantı düşmesini
   açıkça loglamalı.

## Sık tuzaklar

- `unwrap()` / `expect()` üretim kodunda yok. `?` ile yukarı taşı, en üstte logla.
- Win32 pano API'sini farklı thread'lerden çağırma. Pano yazımı **mutlaka** UI
  thread'inde (mesaj kuyruğu üzerinden).
- tokio runtime'ı UI thread'inde blok'lama. `block_on` yalnızca arka plan thread'inde.
- Debug build'de boyut 50+ MB olur — `--release` ile ölç.
- WS reconnect'te eski bağlantının kuyruğundaki mesajı yeni bağlantıya **taşıma**.
  Pending = yalnızca son yerel pano metni (tek slot).
- `tokio-tungstenite` kapanış (close frame) gönderimini düzgün ele almazsa Caddy
  log'unda 1006 görürsün — graceful close gönder.
- Sunucuda mesaj boyut limitini hem `tungstenite` config'inde (max_message_size)
  hem de uygulama seviyesinde uygula. 1 MiB cap.
- Birden fazla istemci aynı `client_id` ile gelirse (örn. aynı makine iki sürüm
  çalıştırıyor) — `client_id` istemcide rastgele üretilmeli (UUID), çakışma olmasın.
- `windows` crate sürümü hızlı değişir, feature isimleri kayabilir; build verirse
  feature listesini docs.rs'ten güncelle.

## Önerilen ilerleme sırası

1. `cargo new --bin client && cargo new --bin server`, üstüne `Cargo.toml`
   workspace'i kur.
2. **Aşama 1 — Sunucu iskeleti**: axum + `/ws` endpoint, token doğrula, echo et.
   `wscat -c "wss://..."` ile el ile test.
3. **Aşama 2 — Oda + broadcast**: iki `wscat` istemcisini aynı odada birbirine
   bağla, broadcast doğrula. `last` cache'i ekle.
4. **Aşama 3 — Sunucu deploy**: Caddy reverse proxy, systemd unit, gerçek domain
   üstünde TLS ile yayında.
5. **Aşama 4 — İstemci UI iskeleti**: tao + tray-icon, hiçbir WS/clipboard logic'i
   yok. Tepsi ikonu görünüyor mu? `--release` boyutu?
6. **Aşama 5 — Pano dinleme**: `AddClipboardFormatListener` + WM_CLIPBOARDUPDATE +
   metni logla.
7. **Aşama 6 — Config**: `config.json` okuma, şablon yazma.
8. **Aşama 7 — WS bağlantısı**: ayrı thread'de tokio runtime, `ws.rs` ile sunucuya
   bağlan, ping/pong, reconnect.
9. **Aşama 8 — Sync motoru**: `sync.rs`, kuyruk + sonsuz döngü engeli + UI
   mesajlaşması.
10. **Aşama 9 — İki makine smoke test**: aynı odada karşılıklı sync, sonsuz döngü
    testi, sunucu yeniden başlatma testi.
11. **Aşama 10 — Cilala**: clippy, README, .gitignore, ikon, sürüm 0.1.0.

## Referanslar

- C# kaynak: `C:\Users\okan\Desktop\netterm\` (GitHub: `ClipSync` repo'su) — yalnızca
  sync mantığı ve sonsuz döngü engeli referansı.
- C# `TrayApp.cs` — `last_local_text` invariant'ı ve döngü engeli.
- `axum` WS örneği: <https://github.com/tokio-rs/axum/tree/main/examples/websockets>
- `tokio-tungstenite` örnekleri: <https://github.com/snapview/tokio-tungstenite/tree/master/examples>
- `tao` örnekleri: <https://github.com/tauri-apps/tao/tree/dev/examples>
- `tray-icon` örnekleri: <https://github.com/tauri-apps/tray-icon/tree/dev/examples>
- `windows` crate feature seçici: <https://microsoft.github.io/windows-rs/features/>
- Caddy reverse_proxy: <https://caddyserver.com/docs/caddyfile/directives/reverse_proxy>
