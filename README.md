# ClipSync

İki makine arasında, kendi sunucunda koşan ufak bir WebSocket relay üzerinden pano
metni senkronize eder. Sistem tepsisinde sessiz çalışır.

```
[PC-A clipsync.exe] ──wss──┐
                           ├──► [VPS: clipsync-server] (ufak relay + son mesaj cache)
[PC-B clipsync.exe] ──wss──┘
```

- **İstemci**: Windows tray uygulaması (`clipsync.exe`, ~3 MB).
  Linux desteği [yol haritası](ClipSync-Rust-WS-Brief.md) listesinde.
- **Sunucu**: Linux'ta tek binary (`clipsync-server`, ~2 MB), Caddy'nin arkasında.

## Hızlı başlangıç

### Sunucu

Detaylı reçete: [`DEPLOY.md`](DEPLOY.md).
Özet: rustup + caddy kur, kaynağı VPS'e gönder, build et, systemd ile koştur,
Caddy'den TLS terminate et.

### İstemci (Windows)

1. `clipsync.exe`'nin yanına `config.json` koy:
   ```json
   {
     "ServerUrl": "wss://clip.example.com/ws",
     "RoomId": "okan-home",
     "Token": "sunucudaki ile aynı uzun rastgele string"
   }
   ```
2. Çift tıkla. Tepside mavi bir **C** ikonu çıkar.
3. İki makineye de aynı yapılırsa biri kopyalayınca diğerinin panosuna anında düşer.

### Tepsi menüsü

- **Durum Bilgisi** — makine adı, sunucu URL'i, oda, bağlantı durumu, son işlem.
  Çift sol tık ikon = aynı pencere.
- **Şimdi Yeniden Bağlan** — WS bağlantısını kapatıp yeniden açar.
- **Çıkış** — uygulamayı sonlandırır.

### Loglama

İstemci tarafında `eprintln!` (dev build'de konsola, release'de yutulur).
Sunucu tarafında `tracing` + systemd journal:

```bash
sudo journalctl -u clipsync-server -f
```

## Tasarım

Wire protokolü, sync mantığı, mimari kararlar, kabul kriterleri ve aşama-aşama
plan için bkz. [`ClipSync-Rust-WS-Brief.md`](ClipSync-Rust-WS-Brief.md).

## Proje yapısı

```
clipsync/
├── Cargo.toml            # workspace
├── client/               # Windows tray app (Rust + tao + tray-icon + tokio-tungstenite)
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs       # giriş + cfg(windows) gate
│       ├── tray.rs       # event loop + tray + status MessageBox
│       ├── clipboard.rs  # AddClipboardFormatListener + read/write
│       ├── ws.rs         # tokio-tungstenite + reconnect + ping/pong
│       ├── sync.rs       # SyncEngine (last_local_text invariant, döngü engeli)
│       ├── config.rs     # config.json (PascalCase)
│       └── log.rs        # (rezerve, dosya logger ileride)
└── server/               # Linux relay (Rust + axum)
    ├── Cargo.toml
    ├── clipsync-server.service
    └── src/
        ├── main.rs       # axum router + graceful shutdown
        ├── ws.rs         # /ws handler + token auth + mpsc per client
        ├── room.rs       # broadcast + son mesaj cache
        ├── wire.rs       # tagged enum mesajlar
        └── config.rs     # config.toml (bind + [[rooms]])
```

## Build

```bash
# Sunucu (Linux)
cargo build --release -p clipsync-server

# İstemci (Windows)
cargo build --release -p clipsync
# → target/release/clipsync.exe
```

## Lisans

(TODO)
