# ClipSync — Deploy Rehberi

İki Windows makine arasında pano senkronizasyonu için merkezde küçük bir WebSocket
relay sunucusu (Linux) ayağa kaldırılır. Bu dosya o sunucuyu kurmanın baştan sona
reçetesidir.

## Ön koşullar

- Bir Linux sunucu (Ubuntu/Debian 22.04+ önerilir; cloud VPS veya ev makinesi olabilir).
- Sunucuya erişebilen bir alan adı, bir A kaydı VPS'in public IP'sine bağlı.
  Örnek: `clip.example.com → 1.2.3.4`. DNSSEC imzalı zone'larda yeni kayıtların
  yayılması 1 saate kadar sürebilir (NSEC3 negative cache).
- Sunucu **ev ağında** ise router'da TCP **80 / 443** portlarını sunucuya
  yönlendir. SSH'ı da farklı bir port'tan (örn. 2222) yönlendirmen yaygın.
- Bazı ISP'ler 80'i bloklar; öyle bir durumda Cloudflare proxy ya da Tailscale
  alternatif.

## Kavramlar: Token ve Room

İkisi de **shared secret** — server ile tüm istemcilerin **birebir aynı string'i**
biliyor olması gerekli. Üretildikleri yer fark etmez (sunucuda, lokalde, kafanda);
önemli olan iki yere de aynı yazılması.

### Token — auth (şifre)

WS bağlantısının kimlik kontrolü. İstemci bağlanırken URL'de
`?token=...` parametresi gönderir, sunucu `config.toml`'da tanımlı token ile
karşılaştırır. Eşleşmezse 401, eşleşirse upgrade.

- **Nasıl üretilir**: rastgele, en az 128-bit entropi. Önerilen:
  ```bash
  openssl rand -hex 32   # 64 karakter, ~256-bit
  ```
- **Nereye gider**:
  - Sunucuda: `/opt/clipsync/config.toml` → `[[rooms]] token = "..."`
  - Her istemcide: `clipsync.exe` yanındaki `config.json` → `"Token": "..."`
- **Niye gizli**: Bu string'i bilen herkes odaya bağlanıp yazılanları
  görebilir. O yüzden config dosyaları `.gitignore`'da, repo'ya commit'lenmemeli.
- **Rotate etmek**: yeni token üret → sunucu config.toml'da güncelle →
  `sudo systemctl restart clipsync-server` → tüm istemcilerin config.json'unu da
  yeni token'la güncelle, uygulamayı yeniden başlat. Eski token'lı istemciler
  bağlanamaz.

### Room — mantıksal kanal

Aynı sunucuyu **birden fazla bağımsız grup** kullanabilsin diye. Bir odadaki
istemciler birbirinin pano'sunu görür; **farklı odadakiler birbirini hiç
görmez**.

- **Senin tipik durumun**: tek bir oda yeterli (örn. `okan-home`,
  `ev`, `kisisel` — istediğin isim). Tüm makineler bu odaya bağlanır.
- **Ne zaman birden fazla**:
  - Arkadaşına aynı sunucunu kullandırırken (ona `ali-evi` odası, ayrı token).
  - İş ↔ kişisel makinelerin panoları karışmasın.
  - Aynı odada test ortamı kuyusu (`test`) ve gerçek (`prod`) ayırmak.
- **Niye token'a ek olarak room**: Token tek başına yeterli auth, ama room
  isolation katmanı. Aynı sunucuda iki ayrı grup, **farklı token + farklı room
  id** ile birbirini hiç fark etmez. (Aynı token + farklı room aslında
  konfigürasyon hatası olur; her oda için farklı token kullan.)

### Üretim ve dağıtım akışı

Bir kere yapılır:

1. Bir tarafa otur (sunucu, lokal makine, fark etmez).
2. `openssl rand -hex 32` ile token üret. Hayatta sadece bu kere göreceksin —
   bir yere kaydet (parola yöneticisine).
3. Bir room adı seç (örn. `okan-home`).
4. Sunucu `config.toml`'una koy:
   ```toml
   [[rooms]]
   id = "okan-home"
   token = "BIR-KEZ-URETTIGIN-TOKEN"
   ```
5. Her Windows istemcisinin `config.json`'una **aynı** ikiliyi koy:
   ```json
   {
     "ServerUrl": "wss://clip.example.com/ws",
     "RoomId": "okan-home",
     "Token": "BIR-KEZ-URETTIGIN-TOKEN"
   }
   ```
6. Sunucu config'i değişince `systemctl restart clipsync-server`. İstemci
   config'i değişince `clipsync.exe`'i yeniden başlat.

Sunucu çoklu oda destekler — `[[rooms]]` bloğunu ekleyerek istediğin kadar
oda açabilirsin, her birine ayrı token:

```toml
[[rooms]]
id = "okan-home"
token = "aaaa..."

[[rooms]]
id = "ali-evi"
token = "bbbb..."   # farklı, paylaşmıyoruz
```

## 1) Sunucuya Rust + Caddy kur

```bash
# rustup (kullanıcı dizinine, sudo gerekmez)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
source ~/.cargo/env

# Caddy resmi reposu
sudo apt-get update
sudo apt-get install -y debian-keyring debian-archive-keyring apt-transport-https curl
curl -1sLf https://dl.cloudsmith.io/public/caddy/stable/gpg.key | \
  sudo gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
curl -1sLf https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt | \
  sudo tee /etc/apt/sources.list.d/caddy-stable.list
sudo apt-get update
sudo apt-get install -y caddy
```

## 2) Sunucuya kaynak gönder + build

Lokalde repo kökünde:

```bash
tar --exclude='target' --exclude='.git' -cz . | \
  ssh user@host -p 2222 'rm -rf ~/clipsync-build && mkdir -p ~/clipsync-build && \
                          tar -xz -C ~/clipsync-build'
ssh user@host -p 2222 '. ~/.cargo/env && \
                       cd ~/clipsync-build && \
                       cargo build --release -p clipsync-server'
```

Sonuç: `~/clipsync-build/target/release/clipsync-server` (~2 MB).

## 3) Service kullanıcısı + dosya yerleştirme

```bash
sudo useradd --system --no-create-home --shell /usr/sbin/nologin clipsync
sudo install -d -o clipsync -g clipsync -m 0755 /opt/clipsync
sudo install -o clipsync -g clipsync -m 0755 \
  ~/clipsync-build/target/release/clipsync-server /opt/clipsync/clipsync-server
```

## 4) Sunucu config'i

> Token ve room mantığı için bkz. [Kavramlar](#kavramlar-token-ve-room).
> Burada üreteceğin token ve seçeceğin room id, Adım 9'da istemci config'inde
> **birebir aynı** olacak.

Güçlü bir token üret:

```bash
openssl rand -hex 32
```

`/opt/clipsync/config.toml`:

```toml
bind = "127.0.0.1:8080"

[[rooms]]
id = "okan-home"        # ya da kendi adlandırman
token = "YUKARIDA-URETILEN-TOKEN"
```

```bash
sudo chown clipsync:clipsync /opt/clipsync/config.toml
sudo chmod 640 /opt/clipsync/config.toml
```

## 5) systemd unit

`/etc/systemd/system/clipsync-server.service`:

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
Group=clipsync
WorkingDirectory=/opt/clipsync
StandardOutput=journal
StandardError=journal
# Sertleştirme
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictNamespaces=true
RestrictRealtime=true
SystemCallArchitectures=native

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now clipsync-server
sudo systemctl status clipsync-server
```

## 6) Caddy reverse proxy + TLS

`/etc/caddy/Caddyfile` (var olan dosyanın yerine):

```caddyfile
{
    email you@example.com
}

clip.example.com {
    reverse_proxy 127.0.0.1:8080
    log {
        output stdout
    }
}
```

```bash
sudo caddy validate --config /etc/caddy/Caddyfile
sudo systemctl reload caddy
```

Caddy ilk istekte Let's Encrypt sertifikası alır (cache: `/var/lib/caddy/...`).
Yenileme otomatik.

## 7) Firewall

```bash
sudo ufw allow 2222/tcp comment "SSH"
sudo ufw allow 80/tcp   comment "HTTP/ACME"
sudo ufw allow 443/tcp  comment "HTTPS/WSS"
sudo ufw enable
```

## 8) Doğrulama

Dışarıdan:

```bash
# TLS chain
openssl s_client -connect clip.example.com:443 -servername clip.example.com </dev/null | \
  openssl x509 -noout -subject -issuer -dates

# WS handshake (wscat ile, npm i -g wscat)
wscat -c "wss://clip.example.com/ws?room=okan-home&token=YUKARIDAKI-TOKEN"
# > {"type":"ping"}
# < {"type":"pong"}
```

## 9) İstemci config'i

`clipsync.exe`'nin yanındaki `config.json`:

```json
{
  "ServerUrl": "wss://clip.example.com/ws",
  "RoomId": "okan-home",
  "Token": "YUKARIDAKI-TOKEN"
}
```

- `RoomId` ve `Token`: Adım 4'tekiyle **birebir aynı**. Tek karakter farkı bile
  401 sebebi olur.
- `ServerUrl`: Adım 6'da kullandığın domain, başına `wss://`, sonuna `/ws`.
- İki (ya da daha fazla) Windows makineye de aynı dosyayı koy. Her birinin
  `clipsync.exe` yanında ayrı `config.json` olmalı.
- Config'i değiştirdiysen `clipsync.exe`'i yeniden başlat (tepsi → Çıkış → çift
  tıkla). Açılışta config oluşturulan klasör, exe'nin bulunduğu klasör.

## Sorun giderme

| Belirti | Sebep | Çözüm |
|---|---|---|
| `Could not connect to 213…:443` | Router 443'ü forward etmiyor ya da ISP 80'i bloklar | Router NAT, ISP destek |
| Caddy log'unda `NXDOMAIN` | DNS henüz yayılmadı | Bekle (1 sa) veya `dig +trace` ile auth NS'i doğrula |
| Caddy log'unda `unauthorized` | Let's Encrypt rate limit | 1 saat sonra dene veya farklı subdomain |
| Client `connection refused` ama 443 erişilebilir | Caddy ya da clipsync-server durdu | `sudo systemctl status caddy clipsync-server` |
| Client bağlanıp 401 | Token uyumsuzluğu | `config.toml` ↔ `config.json` token aynı mı |

## Güncelleme

```bash
# Lokal değişiklik sonrası
tar --exclude='target' --exclude='.git' -cz . | \
  ssh user@host -p 2222 'rm -rf ~/clipsync-build && mkdir -p ~/clipsync-build && \
                          tar -xz -C ~/clipsync-build'
ssh user@host -p 2222 'set -e; . ~/.cargo/env; cd ~/clipsync-build; \
  cargo build --release -p clipsync-server; \
  sudo install -o clipsync -g clipsync -m 0755 \
    target/release/clipsync-server /opt/clipsync/clipsync-server; \
  sudo systemctl restart clipsync-server'
```
