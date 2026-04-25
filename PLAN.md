# BigIris — Plano de Projeto

**Status:** Planejamento inicial · 2026-04-19
**Família:** BigLinux / BigCommunity (prefixo `Big` obrigatório)
**Stack:** Rust 1.83+ · GTK4 · libadwaita · ONNX Runtime
**Alvo primário:** repositórios **BigLinux** e **BigCommunity** (Arch/Manjaro-based)
**Alvos secundários:** Flathub · AUR genérico · GNOME Circle · competições internacionais de UI/UX

---

## 0. Requisitos fundantes (do prompt inicial)

Mapeamento literal do que foi pedido originalmente em `prompt_incio_projeto.md` para as seções do plano. Qualquer desvio requer justificativa explícita.

| # | Requisito original | Onde é atendido |
|---|---|---|
| R1 | Software livre com aparência e recursos **premium** para disputar concursos de UI/UX | §6 (design), §5 (diferenciais M2), licença GPL-3.0-or-later |
| R2 | **Extremamente seguro, resiliente e otimizado** (não consumir recursos à toa) | §10 (segurança), §11 (budget de performance), §3.4 (paralelismo com limite), §3.2 (glycin sandbox) |
| R3 | **Testes automatizados** para evitar regressões; GUI também acessível por **CLI** para CI/CD | §3.6 (CLI espelha GUI), §9 (pirâmide de testes + paridade CLI↔GUI) |
| R4 | **Conversão de imagens** isoladas e em **lote**, redimensionar, girar, cortar — versão moderna do **ReImage** KDE | §5.1 (escopo BigPrisma-módulo), §8 (service menus) |
| R5 | Service menu em **KDE/Dolphin + Nautilus + Nemo + Thunar** | §8.2 (matriz completa, inclui PCManFM-Qt e Pantheon como bônus) |
| R6 | **Rust + GTK4/Adwaita**, libs FOSS mais adequadas | §3.1 (toolkit), §3.2 (decode), §3.3 (transforms) |
| R7 | Precisamos **visualizar ao vivo** os efeitos/cortes/rotações — portanto **visualizador próprio** | §1 (viewer como core do app), §5.2 (funcionalidades), §6.3 (preview) |
| R8 | Visualizador em **Rust + GTK4/Adwaita**, com recursos dos melhores (Gwenview, Shotwell, gThumb, Loupe) | §5.2 (features inventariadas contra os concorrentes) |
| R9 | Visual do visualizador **como Loupe mas mais moderno** — autohide estilo player de vídeo | §5.2 M2, §6.3 (autohide orgânico) |
| R10 | Interface simples para baixar **modelos abertos de IA** (remover fundo, upscale) | §7 (fluxo de download com licença visível) |

---

## 1. Visão

**Um único produto: BigIris.** Um binário `bigiris`, um pacote `bigiris`, uma entrada no menu. Todas as operações (visualizar, converter, redimensionar, girar, cortar, IA) são módulos do mesmo app, acessíveis por três caminhos:

1. **Launcher** (`bigiris`) → abre o visualizador, que é a interface principal.
2. **Service menu / linha de comando com `--dialog=X`** → abre apenas o diálogo modal da operação, sem chrome do viewer. É o caso de uso "clique direito no arquivo" (cf. ReImage).
3. **CLI headless** (`bigiris convert …`) → executa sem GUI, para CI/CD e scripts.

### Por que binário único

- **Velocidade de desenvolvimento**: um codebase, um binário, um pacote, um CI.
- **Preview ao vivo gratuito**: o canvas GPU do viewer é o mesmo usado nos diálogos de edição e IA. Zero duplicação de infraestrutura de render, color management, HDR.
- **Cache de modelo de IA em RAM**: BiRefNet carregado uma vez, reusado por todos os diálogos.
- **Pacote único**: usuário instala `bigiris` e tem tudo. BigLinux/BigCommunity mantém um PKGBUILD.

### Branding interno

O projeto preserva duas identidades de módulo (apenas nomes internos/narrativos, sem impacto arquitetural):

- **Íris** — o módulo de visualização (nome do app).
- **Prisma** — o módulo de conversão/transformação. Aparece em títulos de diálogo ("Prisma — Converter imagem"), em screenshots de marketing e no nome de perfis de conversão exportados.

Narrativa: *"A luz atravessa o Prisma, floresce na Íris."* Permite storytelling em material de divulgação sem fragmentar o produto.

### Princípios inegociáveis

| # | Princípio | Consequência |
|---|---|---|
| 1 | **CLI-first, GUI-em-cima** | Toda operação testável em CI sem display. GUI é cliente do core, não dona da lógica. |
| 2 | **Non-destructive por padrão** | Nunca sobrescrever original sem opt-in explícito. Sufixos (`_png`, `_1080p`) e sidecars. |
| 3 | **GPU opcional, CPU sempre funciona** | Fallback CPU em todo pipeline, incluindo IA. |
| 4 | **Sandbox para decoders** | Usar `glycin` na GUI: decoder por formato isolado em bwrap. |
| 5 | **Licença FOSS pura em modelos IA** | Nenhum peso com cláusula `non-commercial`. Excluir RMBG. Verificar NAFNet por checkpoint. |
| 6 | **Progresso sempre cancelável** | Nenhuma operação longa é bloqueante ou irreversível. |
| 7 | **Acessível AT-SPI/Orca** | Navegação 100% por teclado, alto contraste, scaling fracionário. |
| 8 | **Split-ready**: `bigimage-core` permanece crate brand-agnóstica, para permitir futura separação em múltiplos binários se houver motivo. | Lib interna sem `bigiris`/`bigprisma` no nome. |

---

## 2. Arquitetura de workspace

Monorepo Cargo. Um binário, três libs internas. Core compila em CI sem display.

```
bigiris/                              # diretório do repo (pode ser `bigimage` se preferir monorepo-style)
├── Cargo.toml                        # [workspace]
├── rust-toolchain.toml               # pin Rust stable
├── rustfmt.toml
├── meson.build                       # build de alto nível (gettext, ícones, .desktop, metainfo)
├── meson_options.txt
├── crates/
│   ├── bigimage-core/                # Lógica pura: decode, encode, transform, pipelines. Brand-agnóstica.
│   ├── bigimage-ai/                  # Runtime ort + loaders de modelos. Feature-gated (default=off).
│   ├── bigimage-integrations/        # Gerador de service menus (install/uninstall)
│   └── bigiris/                      # Binário `bigiris` (GTK4 + CLI subcomandos)
├── data/
│   ├── com.biglinux.Iris.desktop.in              # launcher principal (visualizador)
│   ├── com.biglinux.Iris.metainfo.xml.in         # appstream
│   ├── com.biglinux.Iris.gschema.xml             # GSettings schema
│   ├── icons/hicolor/scalable/apps/
│   │   └── com.biglinux.Iris.svg
│   ├── servicemenus/                             # templates KDE/Nemo/Thunar/libfm/Contractor
│   └── ui/                                       # Blueprint (.blp) → .ui
├── po/                                           # i18n: pt_BR primeiro, en_US, es, de, fr
├── build-aux/
│   ├── flatpak/com.biglinux.Iris.yaml            # manifest (alvo secundário)
│   └── archlinux/
│       ├── PKGBUILD
│       └── bigiris.install
├── tests/
│   ├── fixtures/                                 # imagens de referência pequenas
│   ├── golden/                                   # outputs esperados (hash/SSIM)
│   └── integration/                              # testes CLI end-to-end
├── docs/                                         # ADRs, HIG notes, UX studies
├── .github/workflows/ci.yml                      # (ou .gitlab-ci.yml espelhado)
├── .gitignore
├── .editorconfig
├── LICENSE                                       # GPL-3.0-or-later
└── README.md
```

### Regra de dependência

```
bigiris ──→ bigimage-core ──→ (image, glycin, fast_image_resize, libvips opt)
        └─→ bigimage-ai   ──→ ort (feature-gated, CPU default)
        └─→ bigimage-integrations
```

`bigimage-*` não dependem de `gtk4`/`adw`. Isso permite CI headless e preserva rota para split futuro.

---

## 3. Stack técnico — escolhas e justificativas

### 3.1 UI (toolkit)

| Crate | Versão alvo | Função |
|---|---|---|
| `gtk4` + `libadwaita` | 0.11.x / 0.8.x | Toolkit base. Composite templates com **Blueprint** (`.blp`) compilados para `.ui`. |
| `relm4` | 0.10.x | Opcional: só na tela de fila/batch, onde o estado reativo compensa. Resto em gtk4-rs direto. |
| `gettext-rs` | — | i18n, pt_BR default. |

### 3.2 Decode/encode

Estratégia em **três tiers**, inspirada no leque IrfanView (Windows) mas com
gradação clara entre "pure-Rust, sempre on" e "lib C de sistema, opt-in":

**Tier 1 — sempre ligado, pure-Rust + 1 system lib** (via `image` 0.25):

| Formato | Role | Origem |
|---|---|---|
| PNG, JPEG, WebP, TIFF, BMP, GIF | core | pure-Rust |
| ICO, PNM (PPM/PGM/PBM), TGA | legado útil | pure-Rust |
| QOI | moderno niche | pure-Rust |
| HDR (Radiance) | HDR fotográfico | pure-Rust |
| OpenEXR | cinema/VFX, float | pure-Rust |
| **AVIF** | moderno essencial | encode `ravif` + decode `dav1d` (system: `libdav1d`) |

**Tier 2 — opt-in, Cargo features** (cada um traz lib C de sistema):

| Feature | Formato | Lib | Justificativa |
|---|---|---|---|
| `heic` | HEIC/HEIF | `libheif-rs` | Default iPhone; essencial para foto moderna |
| `jxl` | JPEG XL | `jxl-oxide` (decode pure-Rust) + `jpegxl-rs` (encode) | Adobe empurrando; substituto de longo prazo do JPEG |
| `raw` | CR2/CR3, NEF, ARW, RAF, ORF, DNG, PEF, RW2 | `rawler` | Workflow de fotógrafo (decode-only) |

**Tier 3 — roadmap, read-only exóticos à la IrfanView:**

- **PSD** (Photoshop), **XCF** (GIMP), **KRA** (Krita) — read-only.
- **SVG** (vetor) via `resvg` — pipeline separado, render em raster.
- **PDF** multi-página via `pdfium` ou `mupdf` — já previsto em §5.1 M3.

**Caminhos paralelos em outras camadas:**

| Caminho | Escolha | Motivo |
|---|---|---|
| **GUI (preview/visualizador/diálogos)** | **`glycin`** | Backend do Loupe. Loader por formato em **sandbox bwrap**, HDR (CICP). Sobrepõe-se com Tier 1/2 do core — glycin é a primeira opção na GUI, core é fallback. |
| **Metadados** | `kamadak-exif` | EXIF/XMP/IPTC leitura e manipulação. |
| **Fallback batches gigantes** | `libvips` via `libvips` crate | Pipeline streaming, memória constante em 50k+ arquivos. Feature-gate opcional. |

### 3.3 Transformações

| Operação | Status | Crate |
|---|---|---|
| Resize Lanczos3 alpha-aware | **Implementado** (M1) | **`fast_image_resize` 6.x** (SIMD AVX2/NEON) |
| Rotate 90/180/270 (re-encode) | **Implementado** (M1) | `image::imageops` |
| Rotate 90/180/270 lossless JPEG | Adiado pós-M1 (feature `jpeg-lossless`) | `turbojpeg` (system: `libjpeg-turbo`) |
| Rotate ângulo arbitrário (bicubic) | Adiado pós-M1 | `imageproc` |
| Flip H/V | **Implementado** (M1) | `image::imageops` |
| Crop | **Implementado** (M1) | `image::DynamicImage::crop_imm` |
| Color ops (brightness/contrast/saturation/gamma) | M1 pendente | Operações próprias em `core` (SIMD via `wide`) |

### 3.4 Paralelismo

- **CPU-bound**: `rayon` com pool limitado a `num_cpus - 1`.
- **IO/D-Bus/downloads**: `tokio` single-threaded na UI via `glib` MainContext + `spawn_blocking`.
- **Progresso**: `crossbeam::channel` rayon → `glib::idle_add_local_once`.
- **Cancelamento**: `tokio_util::sync::CancellationToken` propagado até cada job.

### 3.5 Inferência de IA

`ort` 2.x (pyke). EPs: CPU (default) · CUDA · ROCm · DirectML · CoreML · OpenVINO. Fallback ordenado. `candle` e `tract` descartados (cobertura/GPU). Opcional futuro: invocar `realesrgan-ncnn-vulkan` via `Command` para rota Vulkan em GPUs AMD/Intel.

### 3.6 CLI — design de entrada única

Binário único com **três modos** determinados por argumentos:

```bash
# Modo 1: Viewer (GUI completa)
bigiris                             # janela vazia
bigiris file.jpg                    # abre visualizador com arquivo
bigiris *.png                       # abre em galeria/film strip

# Modo 2: Diálogo modal (usado por service menus)
bigiris --dialog=convert FILE...     # só o diálogo de conversão
bigiris --dialog=resize  FILE...     # só o diálogo de redimensionar
bigiris --dialog=rotate  FILE...     # só o diálogo de girar
bigiris --dialog=crop    FILE        # só o diálogo de corte
bigiris --dialog=remove-bg FILE...   # só o diálogo de IA remover fundo
bigiris --dialog=upscale  FILE...    # só o diálogo de IA upscale
bigiris --dialog=batch   FILE...     # editor de fila completo

# Modo 3: Headless (CI/CD, scripts, submenu "rápido")
bigiris convert --to png FILE...
bigiris resize  --max-edge 1080 FILE...
bigiris rotate  --degrees 90 FILE
bigiris remove-bg --model birefnet-lite FILE...
bigiris upscale --factor 4 FILE...
bigiris batch --profile my-profile.toml FILE...
bigiris install-integrations [--user|--system]
bigiris uninstall-integrations

# Utilidades
bigiris --self-test                  # smoke test para CI/pós-install
bigiris --help
bigiris --version
```

`clap` v4 derive + `clap_complete` (bash/zsh/fish) + `clap_mangen` (man pages).

### 3.7 Configuração

- **Estado de UI / prefs** → `gio::Settings`, schema `com.biglinux.Iris`.
- **Perfis de conversão** → `serde` + `toml` em `$XDG_CONFIG_HOME/bigiris/profiles/*.toml`.
- **Paths XDG** → `directories` crate.
- **Modelos IA** → `$XDG_DATA_HOME/bigiris/models/`.
- **Logs persistentes** → `$XDG_STATE_HOME/bigiris/log`.
- **Fila de batch recuperável** → `$XDG_STATE_HOME/bigiris/queue.json`.

### 3.8 Observabilidade

`tracing` + `tracing-subscriber` + `tracing-journald`. `#[instrument]` em pipelines, `RUST_LOG=bigiris=debug` ou `RUST_LOG=bigimage_core=trace`. Erros com `color-eyre`.

### 3.9 Empacotamento

**Alvo primário: BigLinux / BigCommunity (Arch-based).** Um único PKGBUILD.

```
build-aux/archlinux/
├── PKGBUILD
└── bigiris.install
```

**Convenções:**

- Pacote: `bigiris`. `arch=('x86_64')`. Licença `GPL-3.0-or-later`.
- `cargo build --release --locked`; `Cargo.lock` commitado.
- Dependências runtime: `gtk4`, `libadwaita`, `glycin`, `dav1d`, `hicolor-icon-theme`. Todas já no BigLinux.
- `optdepends`:
  - `libvips`: backend de batch para milhares de arquivos
  - `onnxruntime`: recursos de IA (remover fundo, upscale)
  - `libheif`: suporte a HEIC/HEIF (Tier 2)
  - `libjxl`: suporte a JPEG XL (Tier 2)
  - `nautilus-python`: integração com Nautilus
- `makedepends`: `rust`, `cargo`, `meson`, `ninja`, `blueprint-compiler`, `gettext`.
- Submit: repo do projeto **e** PR em `community-repository` (BigCommunity) → após validação → `biglinux-package-builds` (BigLinux oficial).

**Esqueleto de PKGBUILD:**

```bash
pkgname=bigiris
pkgver=0.1.0
pkgrel=1
pkgdesc="Visualizador e conversor de imagens moderno, com integração ao gerenciador de arquivos"
arch=('x86_64')
url="https://github.com/xathay/bigiris"
license=('GPL-3.0-or-later')
depends=('gtk4' 'libadwaita' 'glycin' 'dav1d' 'hicolor-icon-theme')
optdepends=(
  'libvips: backend de batch para milhares de arquivos'
  'onnxruntime: recursos de IA (remover fundo, upscale)'
  'libheif: suporte a HEIC/HEIF (Tier 2)'
  'libjxl: suporte a JPEG XL (Tier 2)'
  'nautilus-python: integração com o gerenciador Nautilus'
)
makedepends=('rust' 'cargo' 'meson' 'ninja' 'blueprint-compiler' 'gettext')
source=("$pkgname-$pkgver.tar.gz::$url/archive/v$pkgver.tar.gz")
sha256sums=('SKIP')

build() {
  cd "$pkgname-$pkgver"
  arch-meson build
  meson compile -C build
}

check() {
  cd "$pkgname-$pkgver"
  meson test -C build --print-errorlogs
}

package() {
  cd "$pkgname-$pkgver"
  meson install -C build --destdir "$pkgdir"
}
```

**Alvos secundários:**

1. **Flathub** (`com.biglinux.Iris`). **Atenção:** CPU-only para IA via Flatpak (CUDA/ROCm inviável em sandbox). Documentar.
2. **AUR genérico**: mesmo PKGBUILD no `aur.archlinux.org`.
3. **Debian/Ubuntu** via `cargo-deb` (pós-lançamento).
4. **Fedora** via `cargo-generate-rpm` (pós-lançamento).
5. **Snap**: descartado (sandbox quebra service menus).

---

## 4. `bigimage-core` — API pública

Contrato entre GUI e CLI. Toda operação retorna `Result<Output, BigImageError>`.

```rust
pub struct ConvertJob {
    pub input: PathBuf,
    pub output: PathBuf,
    pub target_format: Format,
    pub transforms: Vec<Transform>,
    pub color: ColorPolicy,
    pub metadata: MetadataPolicy,
    pub overwrite: OverwritePolicy,
}

pub enum Transform {
    Resize { mode: ResizeMode, filter: Filter },
    Rotate { degrees: Rotation },
    Flip { axis: Axis },
    Crop { rect: CropRect },
    Adjust { brightness: f32, contrast: f32, saturation: f32, gamma: f32 },
}

pub enum AiTask {
    RemoveBackground { model: BgModel, alpha_matting: bool },
    Upscale { model: UpscaleModel, factor: u8, tile_size: u32 },
    Denoise { model: DenoiseModel },
}

pub trait Pipeline: Send + Sync {
    fn plan(&self, jobs: &[ConvertJob]) -> PlanReport;
    fn run(&self, jobs: Vec<ConvertJob>, progress: ProgressSink, cancel: CancelToken) -> Summary;
}
```

---

## 5. Escopo funcional — MVP e além

### 5.1 Módulo Prisma (conversão/transformação)

**MVP (M1)**: Converter entre PNG/JPEG/WebP/AVIF/HEIC/JPEG XL/TIFF/BMP · Resize (%, edge máx, exato, fit, fill) · Rotate 90/180/270 lossless + arbitrário bicubic · Flip H/V · Crop · Batch com drag-drop, reordenar, dry-run · Policy de sobrescrita = sufixo por padrão · ICC/EXIF policies (strip GPS em destaque) · Perfis TOML exportáveis aparecem no submenu do FM.

**M2 — IA** (feature `ai` habilitada): Remover fundo (BiRefNet lite/full, U²-Net fallback) · Upscale (Real-ESRGAN x2/x4, Real-CUGAN anime) · Download on-demand com hash + licença visível · Preview ao vivo.

**M3 — avançado**: Denoise (SCUNet) · OCR (PaddleOCR) · Watermark · PDF multi-página, WebP animado, APNG · Scripts de pipeline TOML.

### 5.2 Módulo Íris (visualização)

**MVP (M1)** — paridade com Loupe: Abrir JPEG/PNG/WebP/GIF/SVG/TIFF/BMP + AVIF/HEIC/JXL/RAW via glycin · Zoom/pan GPU, pinch, 1:1, fit, double-tap 100%↔fit · Rotate lossless JPEG, crop, salvar · Slideshow · EXIF/XMP/IPTC viewer · ICC/HDR PQ/HLG · Teclado 100%.

**M2 — diferenciais premium**:
- **Autohide orgânico da UI em idle** (~1.5s fade, cursor somem; reaparece antecipando movimento por aceleração)
- **Film strip** de thumbnails horizontal
- **Theater mode** (fundo escurece sem fullscreen)
- Barra de status contextual (resolução, zoom, câmera, lente, ISO, f/, shutter)
- **Mini-player / PiP** (janela flutuante always-on-top)
- **Comparação lado-a-lado** 2–4 imagens sincronizada
- Gestos: swipe-down-dismiss, duplo swipe = jump 10
- Speed control GIF/APNG/WebP (0.25×–4×, step-by-step)
- OSD contextual estilo mpv
- Color picker sRGB/Display-P3/Rec2020

**M3 — extensibilidade**: D-Bus API · Purpose/XDG share completo · GVfs (SFTP, SMB, WebDAV, MTP) · pHash para duplicatas · Import via gPhoto2.

---

## 6. UI/UX — design para competição

### 6.1 Princípios

- **GNOME HIG estrito** no esqueleto (Adwaita, `AdwHeaderBar`, `AdwToolbarView`, `AdwToastOverlay`).
- **Camada premium** em cima: micro-animações GPU, gestos, autohide, feedback tátil.
- **prefers-reduced-motion** respeitado.
- Tema Adwaita; BigLinux tem temática própria — respeitar.

### 6.2 Loop de feedback

Toda operação destrutiva tem toast com "Desfazer" (10s), histórico visível, log persistente. Modal de confirmação só quando irreversível.

### 6.3 Preview ao vivo integrado (R7)

O canvas do módulo Íris é reutilizado nos diálogos do módulo Prisma. Exemplo: no diálogo "Redimensionar", o preview ocupa 70% da modal e atualiza em tempo real ao arrastar slider — porque o mesmo pipeline GPU do viewer renderiza o resultado. **Esta é a razão técnica principal pela qual viewer+conversor são um único binário.**

### 6.4 Ideias premiáveis (diferenciais de UX)

1. **Autohide orgânico** — aceleração de cursor antecipa reaparecimento.
2. **Comparação sincronizada** — raro em FOSS, obrigatório para fotógrafos.
3. **Download de modelos IA com licença visível** — transparência radical.

### 6.5 Blueprint + Adwaita

Todos os `.ui` em Blueprint (`.blp`). Pipeline `blueprint-compiler` → `.ui` → `gresource`.

---

## 7. IA — implementação

### 7.1 Modelos shipados

| Task | Modelo padrão | Licença | Tamanho | Backup |
|---|---|---|---|---|
| Background removal | **BiRefNet lite FP16** ONNX | MIT | ~100 MB | U²-Net Apache, `u2netp` 4.7 MB |
| Background removal HQ | BiRefNet full FP16 | MIT | ~440 MB | — |
| Upscale foto | **Real-ESRGAN `realesr-general-x4v3`** | BSD-3 | ~5 MB | RealESRGAN_x4plus ~67 MB |
| Upscale anime | Real-CUGAN x2/x3/x4 | MIT | ~4–25 MB | — |
| Denoise (M3) | SCUNet-GAN ONNX | Apache 2.0 | ~90 MB | — |
| OCR (M3) | PaddleOCR v5 det+rec+cls ONNX | Apache 2.0 | ~15–30 MB | Tesseract 5 |

**Excluídos:** BRIA RMBG-1.4/2.0 (non-commercial), CodeFormer (S-Lab NC).

### 7.2 Fluxo de download

1. Pacote nasce **sem modelos**. Manifesto `models.toml` embarcado: `{task, id, url, revision_sha, sha256, size, license_spdx}` com `revision` pinada em commit SHA.
2. Primeira invocação → `AdwAlertDialog`: "Baixar BiRefNet lite (100 MB)?" + licença visível.
3. Streaming, HTTP Range resume, hash SHA-256 streaming (`sha2`).
4. Armazenamento: `$XDG_DATA_HOME/bigiris/models/`.
5. "Gerenciador de modelos" em Preferences.

### 7.3 Runtime

- `ort::Session` em `OnceCell<Arc<Session>>` (uma vez por modelo, reusado por todos os diálogos).
- Warm-up dummy 64×64 no load.
- Fallback de EP: `[CUDA, ROCm, CPU]`.
- Tiling em upscale: `tile_size=256` overlap 32px.
- Queue: um worker por GPU, pool CPU `num_cpus/2`.
- Cancelamento entre tiles e entre imagens.
- UI nunca bloqueia: `spawn_blocking` + canal `mpsc` → main loop.

---

## 8. Integração com file managers (R4, R5)

### 8.1 Estratégia

`bigiris install-integrations [--user|--system]` detecta FMs via `command -v` e instala templates. Idempotente. Desinstala com `bigiris uninstall-integrations`.

Post-install do PKGBUILD **não** instala automaticamente — exibe aviso no `.install` com instruções. Usuário decide.

### 8.2 Matriz de integração

| FM | Mecanismo | Submenu nativo | Local |
|---|---|---|---|
| Dolphin / Konqueror | `.desktop` ServiceMenu + `X-KDE-Submenu` | Sim | `~/.local/share/kio/servicemenus/` |
| Nautilus | **Extensão Python** (`nautilus-python`) com `Nautilus.MenuProvider` **+** árvore de scripts (fallback) | Sim (extensão Python = top-level "Íris ▸"; fallback: Scripts ▸ BigIris ▸) | `~/.local/share/nautilus-python/extensions/bigiris-menu.py` e `~/.local/share/nautilus/scripts/BigIris/` |
| Nemo | `.nemo_action` (um por item) | Não — prefixo `Prisma » …` | `~/.local/share/nemo/actions/` |
| Thunar | `uca.xml` com `<submenu>` | Sim (≥ 4.17) | `~/.config/Thunar/uca.xml` (merge via `xmlstarlet`) |
| PCManFM-Qt / libfm | `.desktop` tipo `Menu` + `Action` | Sim | `~/.local/share/file-manager/actions/` |
| elementary Files | `.contract` | Não | `~/.local/share/contractor/` |

### 8.3 Estrutura do submenu

```
Íris ▸
├── Converter ▸ PNG · JPG · WebP · AVIF · HEIC · JPEG XL · TIFF
├── Redimensionar ▸ 25% · 50% · 1080p · 4K · Personalizado…
├── Girar ▸ 90° · 180° · 270° · Espelhar H · Espelhar V
├── Editar… (abre diálogo Prisma com a seleção)
├── Remover fundo (IA)
├── Fazer upscale (IA) ▸ 2× · 4×
└── Visualizar em Íris
```

Cada item resolve para `bigiris --dialog=X %F` ou `bigiris <subcomando> ... %F` (headless com notify-send).

### 8.4 Invocação

- Operação unívoca ("Converter para PNG"): `bigiris convert --to png %F`, direto, notificação ao fim.
- Operação paramétrica ("Personalizado…", IA): `bigiris --dialog=convert %F` abre modal única.
- Argv padrão. Fallback `--stdin0` NUL-separated para `ARG_MAX`.
- Aceitar `file://`, `smb://`, `sftp://` via GIO.

### 8.5 Flatpak (secundário)

Service menus → `flatpak run com.biglinux.Iris --file-forwarding @@u %U @@`. Recomendação: pacote nativo BigLinux primeiro, Flatpak secundário.

---

## 9. Testes

### 9.1 Pirâmide

```
              /\
             /e2e\           5% — scripts disparam CLI, comparam saídas
            /──────\
           / integ. \        15% — core + ai com fixtures, paridade CLI↔GUI
          /──────────\
         /  unit-core  \     60% — bigimage-core headless, rápido
        /────────────────\
       /    proptest      \  20% — invariantes
      /────────────────────\
```

### 9.2 Técnicas

- **Golden**: SSIM > 0.99 ou `blake3` determinístico para lossless.
- **Proptest**: rotate 4× = id, resize 100% = id, round-trip lossless preserva pixels, crop dentro dos bounds nunca panica.
- **Metadata snapshot**: EXIF out == EXIF in quando preserve.
- **GUI**: `#[gtk::test]` para widgets custom; smoke via `bigiris --self-test` no post-install.
- **Paridade CLI↔GUI**: cada teste roda a operação pela API e pela CLI; quebra se divergir.
- **Bench**: `criterion`, regressão > 15% falha.
- **Fuzz**: `cargo-fuzz` nightly.

### 9.3 Matriz CI

| Job | Gatilho | Duração-alvo |
|---|---|---|
| fmt + clippy --all-targets -D warnings | PR | < 1 min |
| core unit + proptest | PR | < 3 min |
| integration CLI | PR | < 5 min |
| container Arch: `makepkg` | PR | < 10 min |
| flatpak build + smoke | PR | < 15 min |
| criterion bench | PR | < 10 min |
| fuzz 24h | nightly/weekly | — |

---

## 10. Segurança e resiliência (R2)

| Vetor | Mitigação |
|---|---|
| Decoder CVE (libjpeg/libpng/libheif) | **glycin sandbox** por loader (bwrap, syscall-restricted). Pure-Rust onde possível. |
| Path traversal em output | Canonicalizar, refutar fora do dir escolhido. |
| Download MITM de modelo | TLS + SHA-256 + `revision_sha` pinado. Falha → abort. |
| Sobrescrita acidental | Padrão sufixo, replace exige confirmação. |
| OOM em batch grande | libvips streaming + tiling + limite por RAM livre detectada. |
| GPS em EXIF (LGPD) | Policy "strip GPS" em destaque, alerta se presente antes de publicar. |
| Crash perde fila | Persistida em `$XDG_STATE_HOME/bigiris/queue.json` a cada commit. |
| Crash em módulo derruba tudo (risco do binário único) | Threading: rayon para CPU, isolamento de panics na boundary do core. Diálogos custam < 5ms para reabrir, UX aceitável. |
| Modelo IA não-livre | CI valida `license_spdx` contra allowlist (`MIT, Apache-2.0, BSD-3-Clause, BSD-2-Clause, MPL-2.0`). |

---

## 11. Performance — budget (R2)

| Operação | Alvo |
|---|---|
| Startup até primeira imagem | < 350 ms cold, < 100 ms warm |
| Decode JPEG 24 MP | < 150 ms |
| Resize JPEG 24 MP → 1080p (Lanczos3) | < 60 ms em AVX2 |
| Convert 1000 JPEGs 4 MP → WebP (8c) | < 90 s |
| Remover fundo 1080p BiRefNet-lite CPU | < 6 s |
| Remover fundo 1080p BiRefNet-lite CUDA | < 0.3 s |
| Upscale 512→2048 Real-ESRGAN x4v3 CPU | < 8 s |
| Memória residente idle | < 120 MB |

Validado em CI com `criterion`.

---

## 12. Roadmap

### M0 — Fundação ✅ Concluído

- Scaffold workspace Cargo + CI básico.
- ADR-001 / ADR-002 / ADR-003.
- `bigimage-core`: tipos públicos.
- `bigiris convert --to png FILE` funcional.
- `bigiris FILE` abre janela GTK vazia.
- **PKGBUILD** completo (principal + `.local` para dev).

### M1 — MVP ✅ Concluído (testável)

Entregue nesta iteração (85 testes verdes):

**CLI** — `bigiris convert/resize/rotate/flip/crop/adjust/install-integrations/uninstall-integrations`, com `--overwrite skip|replace|increment` em todas as operações de arquivo.

**Formatos Tier-1 (13)** — PNG, JPG, WebP, AVIF (encode ravif + decode dav1d), TIFF, BMP, GIF, ICO, PNM, TGA, QOI, HDR, OpenEXR. Conversão automática de color type para EXR/HDR. Cleanup de arquivo parcial em falha.

**Transformações**:
- Resize — 4 modos (MaxEdge, Percent, Exact, Fit) × 5 filtros (Lanczos3 default, Mitchell, CatmullRom, Bilinear, Nearest) via `fast_image_resize` 6.x.
- Rotate — 90/180/270 via `image::imageops`.
- Flip — horizontal/vertical.
- Crop — retângulo com validação de bounds + overflow u32.
- **Adjust** — brilho/contraste/saturação/gamma (convenção Photoshop/GIMP).

**Viewer (módulo Íris)** — GTK4 + libadwaita com:
- `gtk::Picture` em `gtk::ScrolledWindow`, zoom cursor-anchored, drag pan (qualquer botão).
- Teclado: +/−/0/1 zoom, ←/→/Home/End/Space/PgUp/PgDn nav, F11 fullscreen, Esc quit.
- `ZoomState { Fit | Scale(f64) }` com `ContentFit::Contain`/`Fill` switch automático.
- Reset zoom ao trocar de arquivo.
- AppID `com.biglinux.Iris` + `AdwStatusPage` para estado vazio.

**Diálogos modais Prisma (5)** — `--dialog=convert|resize|rotate|flip|adjust`, lançados pelo submenu "Personalizar…" ou via CLI. Execução via `idle_add_local_once` (UI responsiva), auto-close em sucesso.

**Integrações com 6 gerenciadores de arquivos**:
- Dolphin / Konqueror (`.desktop` + `X-KDE-Submenu` aninhado "Íris/Converter")
- Nautilus (extensão Python `nautilus-python` top-level **+** árvore de scripts fallback)
- Nemo (`.nemo_action` com prefixo "Íris ▸")
- Thunar (merge em `~/.config/Thunar/uca.xml` preservando ações de terceiros)
- PCManFM-Qt / libfm (`.desktop` `Type=Action`)
- elementary Files (`.contract`)

Submenu "Íris ▸" com 6 subgrupos (Converter · Redimensionar · Girar · Espelhar · Ajustar cores · Visualizar) e "Personalizar…" em Converter/Redimensionar/Girar/Espelhar/Ajustar.

**Empacotamento** — PKGBUILD principal (`git+https` source) + `PKGBUILD.local` (source local sem push). Instalação system-wide via `bigiris install-integrations --system --destdir=$pkgdir` chamado pelo `package()`. `.install` hook refresca caches (glib-schemas / desktop / icon / mime / kbuildsycoca).

### M1 — pendente (antes de fechar M1 oficial)

- Golden tests com SSIM > 0.99, proptests.
- Paridade Loupe: EXIF viewer, slideshow, ICC/HDR PQ/HLG.
- Submit na BigCommunity.

### M2 — Premium + IA (Semana 7–12)

- Íris M2: autohide, film strip, theater, OSD, gestos, color picker.
- Preview ao vivo nos diálogos do Prisma usando o canvas do Íris.
- IA: BiRefNet lite + Real-ESRGAN x4v3.
- Download manager com hash + licença visível.
- Submit para **repositório oficial BigLinux** após validação.

### M3 — Flathub + Circle (Semana 13–14)

- HIG review interno.
- Traduções pt_BR + en_US + es.
- Screenshots + metainfo AppStream.
- Submissão Flathub.
- Aplicação ao GNOME Circle.

### M4 — Pós-lançamento

- Comparação lado-a-lado, PiP, D-Bus API.
- Denoise (SCUNet), OCR (PaddleOCR).
- GVfs remotos.
- Scripts TOML user-defined.

---

## 13. Decisões em aberto (ADRs a escrever)

1. **Relm4 onde?** Tela de fila apenas. _Proposta aceita._
2. **Vulkan via ncnn?** Descartar M2, reavaliar M4.
3. **Nome do repo Git** — `xathay/bigiris` (host pessoal de Leonardo Athayde, sem acesso admin no `biglinux`). _Decidido em 2026-04-25._
4. **Reverse-DNS** — `com.biglinux.Iris` ou `br.com.biglinux.Iris`? Confirmar com equipe BigLinux.
5. **Licença** — GPL-3.0-or-later (proposta). Confirmar.
6. **Assinatura** — minisign no repo, GPG em tags.
7. **Renomeação do diretório local** de `BigFM/` para `bigiris/` — cosmético, pós-scaffold.

---

## 14. Riscos e mitigações

| Risco | Impacto | Mitigação |
|---|---|---|
| ORT + CUDA inviável em Flatpak | Alto | Flatpak = CPU-IA; GPU só em native BigLinux. Documentado. |
| glycin não roda em CI minimal | Médio | CI pure-Rust no core; glycin só em smoke container. |
| Submenu falso no Nemo feio | Baixo | Alternativa: zenity/yad como pseudo-menu. |
| Binário único fica inchado (risco da nova arquitetura) | Médio | Lazy-load de módulos GTK pesados (IA widgets só quando abertos); feature-gate `ai` no Cargo.toml. |
| Download HF fora | Médio | Mirror S3/R2 env-configurável; BigLinux pode hospedar. |
| Licença de modelo muda | Alto | Pin `revision_sha` + snapshot em mirror. |
| GTK4 API break 48→50 | Baixo | Alvo = stable no BigLinux vigente; bump controlado. |
| Circle lento | Médio | Não é bloqueio. BigLinux/Flathub primários. |
| PKGBUILD reprovado na BigCommunity | Médio | Seguir template oficial, container clean test, revisão prévia de mantenedor. |

---

## 15. Próximos passos imediatos

1. Confirmar reverse-DNS com equipe BigLinux (`com.biglinux.*` vs `br.com.biglinux.*`).
2. Confirmar licença (proposta: GPL-3.0-or-later).
3. Criar repo `xathay/bigiris`.
4. Renomear diretório local `BigFM/` → `bigiris/` (pós-scaffold, cosmético).
5. Scaffold Cargo workspace + meson + ci.
6. ADR-001, ADR-002, ADR-003.
7. `.desktop`, metainfo AppStream, ícone SVG placeholder.
8. PKGBUILD esqueleto em `build-aux/archlinux/` — `makepkg -si` em Arch limpo mesmo com binário "Hello".
9. CI fmt + clippy + `makepkg` em container.

Arquitetura fechada: **um produto, um binário, um pacote — com módulos Íris (visualizar) e Prisma (transformar) compartilhando canvas, IA e core.**
