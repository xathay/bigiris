<p align="center">
  <img src="data/icons/hicolor/scalable/apps/com.biglinux.Iris.svg" alt="BigIris" width="160">
</p>

<h1 align="center">BigIris</h1>

<p align="center">
  <strong>Visualizador e conversor de imagens moderno para Linux.</strong><br>
  Rust + GTK4/libadwaita · IA local que nunca sai do seu computador · single-binary com CLI e GUI.
</p>

---

Substitui o ServiceMenu *ReImage* (KDE) e atualiza o estado da arte de viewers como Loupe e Gwenview num único app, com dois módulos:

- **Íris** — visualizar (zoom cursor-anchored, drag pan em qualquer botão, navegação, fullscreen, slideshow, film strip, histograma, modo teatro).
- **Prisma** — transformar (converter, redimensionar, girar, espelhar, cortar, ajustar cores, comparar, metadados, animar, lote, IA).

## O que faz diferente

### Performance

**Batch paralelo no nível do IrfanView.** Cada diálogo Prisma fan-outa o trabalho num pool de worker threads (= `num_cpus − 1`, deixa um core pra GUI) coordenado por `AtomicUsize` sem Mutex no caminho quente. Numa CPU 16-thread, 100 AVIFs → JPEG terminam em segundos; janela arrastável, progresso ao vivo, cancelável mid-batch sem GTK marcar "Não respondendo". Implementação em [`crates/bigiris/src/gui/batch_runner.rs`](crates/bigiris/src/gui/batch_runner.rs).

**Encoder rápido por padrão.** AVIF emitido com ravif `speed=8` (vs. default=4 do crate `image`) — ~3× mais rápido sem perda visual perceptível. PNG default usa compressão padrão; JPEG quality controlável pelo spinner. O toggle **"Otimizar tamanho"** no diálogo Avançado força speed=4 + PNG `Best` quando o usuário troca tempo por tamanho.

**Allocator multi-thread.** `mimalloc` como `#[global_allocator]` — heap por thread sem lock contention nas alocações que o batch faz por arquivo (decode → RGBA buffer → encode). 5-15% extra de wall-clock em CPUs ≥ 8 cores.

**Stats pós-batch.** Quando o lote termina, o status row mostra tempo total + bytes antes/depois + médias por arquivo:

```
Concluído em 8.2 s · 23 gravado(s), 0 ignorado(s), 0 falha(s)
8.5 MB → 1.2 MB (−86%) · média por arquivo 378 KB → 53 KB
```

**Memória constante em batches grandes.** Um arquivo decodificado por vez por worker; encoder fecha o anterior antes do próximo abrir. SIMD para resize via [`fast_image_resize`](https://docs.rs/fast_image_resize). Decode caps preventivos (1 GiB / 256 MP) refusam pixel bombs antes da descompressão.

### Privacidade & segurança

**IA local de verdade.** Remover fundo com **BiRefNet-lite** (MIT) roda 100% offline. A imagem nunca sai do disco — sem upload, sem conta, sem API, sem créditos. Modelo (~224 MB) baixado uma única vez do mirror oficial da comunidade ONNX, **verificado por SHA-256** contra o hash gravado no binário. Allowlist FOSS rígida (MIT/Apache-2.0/BSD/MPL-2.0/CC0); pesos com cláusula *non-commercial* recusados antes do download. Sessão ONNX cacheada entre arquivos do mesmo lote — modelo carrega 1× pra batch de N arquivos. Detalhes em [docs/IA-LOCAL.md](docs/IA-LOCAL.md).

**EXIF/GPS strip-by-default.** `EncodeOptions::strip_metadata = true` é o contrato do encode. O pipeline já dropa EXIF/IPTC/XMP/GPS implicitamente (decode → `DynamicImage` → encode), e o flag explícito existe pra que um futuro toggle "preservar" nunca regrida silenciosamente.

**Endurecido contra entrada hostil.**

- **Decode caps** — arquivos > 1 GiB e imagens > 256 MP rejeitados antes de descomprimir.
- **Download cap** — mirror enviando > 16 MiB acima do esperado aborta com cleanup do `.part`.
- **`O_NOFOLLOW`** em todas as escritas dos service menus — `sudo install-integrations --system` não segue symlinks plantados.
- **Thunar UCA** empacotado em `sh -c '… "$@"' progname %F` — filenames jamais reinterpretados pelo shell.
- **TLS** via `native-tls` (OpenSSL do sistema), sem `ring` no caminho de build, sem `danger_*` desativando validação.
- **`#![forbid(unsafe_code)]`** em todos os crates.

Política de reporte em [SECURITY.md](SECURITY.md).

### UX & integração

**Single-binary, módulos por feature flag.** Um executável carrega CLI, viewer GTK4 e diálogos Prisma. IA atrás do feature `ai`; integrações de file manager geradas pelo próprio binário (`bigiris install-integrations --system|--user`).

**CLI espelhada da GUI.** `bigiris convert ...`, `bigiris resize ...`, `bigiris remove-bg ...`, `bigiris adjust ...`. CI/CD testa exatamente o que o usuário usa.

**6 gerenciadores integrados.** Dolphin, Nautilus (extensão Python top-level), Nemo, Thunar (merge `uca.xml`), PCManFM-Qt, elementary Files. Clique direito em qualquer imagem:

```
BigIris ▸  Visualizar em BigIris                  ← top: 1-clique pra abrir
           Converter      ▸ PNG · JPG · WebP · AVIF · TIFF · Mais opções…
           Redimensionar  ▸ 25% · 50% · 200% · HD · 4K · Mais opções…
           Girar          ▸ 90° · 180° · 270° · Auto (EXIF) · Mais opções…
           Espelhar       ▸ Horizontal · Vertical · Mais opções…
           Ajustar cores  ▸ Brilho ± · Contraste + · P&B · Vivas · Mais opções…
           Para web       ▸ WhatsApp · Instagram · Facebook · Twitter · Telegram · Discord · PNG otim.
           Metadados      ▸ Ver · Remover tudo (re-encode limpo)
           Utilidades     ▸ Lote · GIF animado · Comparar
           IA             ▸ Remover fundo (BiRefNet)
           PDF            ▸ Converter (LibreOffice headless)  ← em documentos
```

**Diálogos Prisma seguem GNOME HIG.** `PreferencesGroup` por seção, `ExpanderRow` "Avançado" para opções de nicho (JPEG progressivo, otimização do encoder), picker de pasta de saída opcional. Tela primária fica em **Formato → Qualidade → Destino**.

**13 formatos Tier-1 nativos** — PNG, JPG, WebP, **AVIF (default)**, TIFF, BMP, GIF, ICO, PNM, TGA, QOI, HDR, OpenEXR. HEIC, JPEG XL e RAW por trás de feature flags.

**i18n com gettext.** Source pt-BR, catálogos `data/po/<lang>.po`. `data/po/en.po` cobre 90 msgids de UI; helper `data/po/regen-pot.sh` regenera o template via `xgettext` e propaga via `msgmerge`.

## Status

#### Funcionalidade

| Bloco | Estado |
|---|---|
| CLI completo: `convert` · `resize` · `rotate` · `flip` · `crop` · `adjust` · `remove-bg` · `upscale` · `animate` · `to-pdf` · `install-integrations` | ✓ |
| Viewer GTK4: zoom cursor-anchored · drag pan · navegação · fullscreen · slideshow · film strip · histograma · modo teatro | ✓ |
| 12 diálogos Prisma: convert · resize · rotate · flip · crop · adjust · upscale · animate · batch · compare · metadata · remove-bg | ✓ |
| 13 formatos Tier-1 (PNG · JPG · WebP · AVIF · TIFF · BMP · GIF · ICO · PNM · TGA · QOI · HDR · EXR) | ✓ |

#### Performance

| Bloco | Estado |
|---|---|
| Worker pool paralelo (cores − 1) em todos os batches Prisma + Animate; UI nunca freezea | ✓ |
| `mimalloc` global allocator + AVIF speed=8 default (clamp pra 4 quando "Otimizar") | ✓ |
| Stats pós-batch: tempo · bytes in/out · % delta · média por arquivo | ✓ |
| Sessão ORT cacheada entre arquivos do mesmo lote; intra-threads capadas | ✓ |
| SIMD para resize via `fast_image_resize`; decode caps (1 GiB / 256 MP) | ✓ |

#### Segurança

| Bloco | Estado |
|---|---|
| `#![forbid(unsafe_code)]` em todos os crates · SPDX em todo source | ✓ |
| Decode caps · download cap (16 MiB margem) · `O_NOFOLLOW` no install · Thunar shell quoting | ✓ |
| EXIF/GPS strip-by-default · TLS via `native-tls` (sem ring) | ✓ |
| FOSS license allowlist + SHA-256 nos modelos IA | ✓ |
| `SECURITY.md` · `CONTRIBUTING.md` · GitHub Security Advisories habilitado | ✓ |

#### Empacotamento

| Bloco | Estado |
|---|---|
| 6 gerenciadores de arquivos integrados · Nautilus detecta system-wide e evita menu duplicado | ✓ |
| AppStream metainfo · `.desktop` validado · ícone hicolor SVG · `<screenshots>` + `<releases>` | ✓ |
| i18n: 90 msgids em pt-BR (source) + `en.po` completo | ✓ |
| PKGBUILD principal (`xathay/bigiris`) + `.local` (build do checkout) | ✓ |
| CI: 9 jobs (fmt · 2× clippy · 2× test · build release · AppStream · desktop · makepkg) · 141 testes | ✓ |

#### Roadmap (M2)

| Bloco | Estado |
|---|---|
| Glycin sandboxed decode · MDI · EXIF sidebar · preview ao vivo | em planejamento |

## Remover fundo com IA — local, privado, sem conta

```bash
bigiris remove-bg foto.jpg              # CLI → foto_nobg.png (RGBA)
bigiris --dialog=remove-bg foto.jpg     # diálogo GUI com progress + compare
bigiris remove-bg *.jpg                 # lote (sessão ORT cacheada entre arquivos)
```

Na primeira execução, o modelo (~224 MB) é baixado uma única vez do [mirror oficial da comunidade ONNX](https://huggingface.co/onnx-community/BiRefNet_lite-ONNX) para `~/.local/share/iris/models/` e **verificado por SHA-256** contra o hash fixado no binário. Qualquer divergência aborta — um mirror comprometido não consegue injetar pesos alterados. Tamanho > expected + 16 MiB também aborta, pra não encher o disco. Chamadas seguintes usam o cache local.

Documentação completa: [docs/IA-LOCAL.md](docs/IA-LOCAL.md).

## Como instalar

### Opção A — `makepkg` (Arch / BigLinux / Manjaro)

A partir do diretório `pkgbuild/`:

```bash
cd pkgbuild
makepkg -si                        # builda do remoto (git+https://github.com/xathay/bigiris)
makepkg -si -p PKGBUILD.local      # ou builda do checkout local, pula o clone
```

O hook `.install` informa que as integrações **já estão ativas system-wide** após a instalação — clique direito em qualquer imagem e o submenu "BigIris ▸" aparece nos seis gerenciadores.

Para desinstalar: `sudo pacman -R bigiris`.

### Opção B — `cargo install` (qualquer distro)

```bash
# Dependências de sistema (Manjaro/BigLinux):
sudo pacman -S --needed gtk4 libadwaita dav1d openssl gettext hicolor-icon-theme rust pkgconf

# Da raiz do repo:
cargo install --path crates/bigiris --features "gui ai" --locked

# Garante PATH:
export PATH="$HOME/.cargo/bin:$PATH"

# Instala integrações de clique direito no seu usuário:
bigiris install-integrations --user
```

Para desinstalar: `bigiris uninstall-integrations && cargo uninstall bigiris` (rode o `uninstall-integrations` **antes** do `cargo uninstall`, senão o binário some).

### Opção C — `cargo run` (dev rápido, sem instalar)

```bash
cargo build --release --locked --features "gui ai" -p bigiris
./target/release/bigiris foto.jpg                   # viewer
./target/release/bigiris --dialog=convert foto.jpg  # diálogo Prisma
```

## Roteiro de teste em 5 minutos

```bash
# (a) Self-test — valida o binário
bigiris --self-test

# (b) CLI puro
bigiris convert --to avif minha-foto.jpg
bigiris resize --max-edge 1080 minha-foto.jpg
bigiris rotate --degrees 90 minha-foto.jpg
bigiris flip --axis horizontal minha-foto.jpg
bigiris crop --rect 800x600+100+50 minha-foto.jpg
bigiris adjust --brightness 15 --contrast 10 minha-foto.jpg

# (c) Viewer
bigiris foto.jpg *.png
#   Teclado: +/- (zoom), 0 (ajustar), 1 (1:1), ←/→ (prev/next),
#            Home/End, Space/Backspace, F11 (fullscreen), Esc
#   Mouse:   scroll = zoom centrado no cursor
#            drag (qualquer botão) = pan quando zoomed in

# (d) Diálogos modais Prisma
bigiris --dialog=convert foto.jpg
bigiris --dialog=resize foto.jpg

# (e) Integração com file manager
#     Clique direito numa imagem → submenu BigIris ▸

# (f) Batch IrfanView-style — selecione 100 fotos no file manager
#     → BigIris ▸ Converter ▸ JPG → "Convertendo 12/100 — foto.jpg"
#     → janela arrastável, cancelável, sem freeze
```

## CLI completa

```bash
# Transformações headless (todas com cap de 1 GiB / 256 MP no decode)
bigiris convert --to png foto.jpg
bigiris convert --to avif --overwrite replace *.jpg
bigiris resize --max-edge 1080 foto.png
bigiris resize --percent 50 --filter lanczos3 foto.png
bigiris resize --exact 800x600 --to webp foto.png
bigiris resize --fit 3840x3840 foto.png          # upscale+downscale preservando aspecto
bigiris rotate --degrees 90 foto.jpg
bigiris rotate --auto --overwrite increment foto.jpg  # EXIF Orientation
bigiris flip --axis horizontal foto.jpg
bigiris crop --rect 800x600+100+50 foto.jpg
bigiris adjust --brightness 15 --contrast 10 foto.jpg
bigiris adjust --saturation -100 foto.jpg        # preto e branco
bigiris adjust --gamma 0.7 foto.jpg              # clareia midtones

# IA local (BiRefNet-lite MIT, offline, sessão cacheada entre arquivos)
bigiris remove-bg foto.jpg                       # → foto_nobg.png (RGBA)
bigiris remove-bg *.jpg                          # lote

# Upscale (Lanczos3 CPU hoje; Real-ESRGAN planejado)
bigiris upscale --factor 2 foto.jpg              # 2x, 3x ou 4x

# GIF animado
bigiris animate -o saida.gif --delay 100 frame_*.png

# Documentos → PDF (via LibreOffice headless)
bigiris to-pdf documento.docx planilha.xlsx apresentacao.pptx

# Diálogos modais Prisma (usados pelo "Mais opções…" dos menus)
bigiris --dialog=convert *.jpg
bigiris --dialog=resize *.png
bigiris --dialog=rotate foto.jpg
bigiris --dialog=flip foto.jpg
bigiris --dialog=adjust foto.jpg
bigiris --dialog=remove-bg foto.jpg
bigiris --dialog=batch *.jpg
bigiris --dialog=animate frame_*.png

# Viewer
bigiris foto.jpg                  # janela com imagem
bigiris                           # janela vazia
bigiris *.png                     # galeria (← → para navegar)

# Integrações
bigiris install-integrations --user
bigiris install-integrations --system --destdir=/tmp/stage  # para packaging
bigiris uninstall-integrations

# Debug
bigiris --self-test
bigiris --version
```

## Arquitetura

```
bigiris/
├── crates/
│   ├── bigimage-core/             # Decode/encode/transforms — pure Rust, headless
│   │   └── pipeline.rs            # decode caps (1 GiB / 256 MP), strip-metadata default
│   ├── bigimage-ai/               # ORT + modelos (feature `onnx`)
│   │   ├── background.rs          # BgSession cacheada + vectorized pre/post-process
│   │   └── download.rs            # SHA-256 + FOSS allowlist + size cap (16 MiB margin)
│   ├── bigimage-integrations/     # Service menus para 6 file managers
│   │   ├── safe_fs.rs             # O_NOFOLLOW write helper (anti-symlink-follow)
│   │   └── thunar.rs              # UCA com sh -c '... "$@"' (anti-shell-injection)
│   └── bigiris/                   # Binário único (CLI + GUI via feature `gui`)
│       └── src/gui/
│           └── batch_runner.rs    # run_batch_async + AsyncBatchEvent (worker threads)
├── data/
│   ├── com.biglinux.Iris.{desktop,metainfo.xml,gschema.xml}
│   ├── icons/hicolor/scalable/apps/com.biglinux.Iris.svg
│   ├── nautilus/bigiris-menu.py   # nautilus-python extension
│   └── po/                        # i18n: bigiris.pot, en.po, LINGUAS, regen-pot.sh
├── pkgbuild/
│   ├── PKGBUILD                   # build do git remoto
│   ├── PKGBUILD.local             # build do checkout (usado pela CI)
│   └── bigiris.install            # _refresh_caches: gtk-update-icon-cache, kbuildsycoca, etc.
└── docs/
    ├── ADR-00{1,2,3}-*.md         # decisões arquiteturais
    ├── IA-LOCAL.md                # detalhes da IA local
    └── screenshots/               # PNGs referenciados pelo AppStream metainfo
```

ADRs explicam *por que* a estrutura está assim — `ADR-001` separa core de GUI/CLI, `ADR-002` define glycin para GUI + pure-rust para CLI, `ADR-003` justifica single-binary multi-module.

## Desenvolvimento

Requer Rust stable ≥ 1.83 e libs de sistema (`gtk4`, `libadwaita`, `dav1d`, `openssl`, `pkgconf`, `gettext`).

```bash
# Build sem GUI (CI headless, compila rápido)
cargo build

# Build com viewer GTK4 + IA
cargo build --features "gui ai" -p bigiris

# Qualidade — gates obrigatórios no CI
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p bigiris --features "gui ai" --all-targets -- -D warnings
cargo test --workspace
cargo test -p bigiris --features "gui ai"

# i18n: regen .pot + msgmerge nos catálogos depois de adicionar gettext("…")
data/po/regen-pot.sh
```

CI executa em GitHub Actions: `fmt`, `clippy (headless)`, `clippy (gui+ai)`, `test (headless)`, `test (gui+ai)`, `build (release, gui+ai)` com `--self-test`, `appstreamcli validate`, `desktop-file-validate`, `makepkg` em container Arch. Veja [`.github/workflows/ci.yml`](.github/workflows/ci.yml).

Para contribuir: leia [CONTRIBUTING.md](CONTRIBUTING.md). Para reportar vulnerabilidades: **não abra issue pública**, veja [SECURITY.md](SECURITY.md).

## Licença

GPL-3.0-or-later. Ver [LICENSE](LICENSE). Cada arquivo `.rs` / `.py` começa com `// SPDX-License-Identifier: GPL-3.0-or-later` para auditoria distro (Debian, Fedora).

Parte da família **BigLinux**. Mantido por Leonardo Athayde.
