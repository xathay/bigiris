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

- **Íris** — visualizar (zoom cursor-anchored, drag pan em qualquer botão, navegação, fullscreen).
- **Prisma** — transformar (converter, redimensionar, girar, espelhar, ajustar cores, IA).

## O que faz diferente

**Privacidade real, não slogan.** Remover fundo com **BiRefNet-lite** (licença **MIT**) roda 100% offline. A imagem nunca sai do disco — sem upload, sem conta, sem API externa, sem créditos. Detalhes em [docs/IA-LOCAL.md](docs/IA-LOCAL.md).

**Cadeia de confiança verificável.** Pesos pinados por **SHA-256** contra o hash gravado no binário. Allowlist FOSS rígida (MIT, Apache-2.0, BSD, MPL-2.0, CC0); modelos com cláusula *non-commercial* escondida são recusados antes do download (ver [`crates/bigimage-ai/src/download.rs`](crates/bigimage-ai/src/download.rs)).

**Stack 2026.** Rust stable (≥ 1.83), GTK4, libadwaita. Decodificação sandboxed via `glycin` (M2). SIMD para resize, memória constante em batches de milhares de arquivos.

**Toda funcionalidade tem CLI espelhada da GUI.** `bigiris convert ...`, `bigiris resize ...`, `bigiris remove-bg ...`, `bigiris adjust ...`. CI/CD testa exatamente o que o usuário usa — sem regressão de feature por divergência de path.

**Single-binary, módulos por feature flag.** Um executável carrega CLI, viewer GTK4 e diálogos Prisma. IA atrás do feature `ai`; integrações de file manager geradas pelo próprio binário.

**Integração nativa em 6 gerenciadores.** Dolphin, Nautilus (extensão Python top-level), Nemo, Thunar (merge `uca.xml`), PCManFM-Qt, elementary Files. Clique direito em qualquer imagem mostra:

```
Íris ▸  Converter      ▸ PNG · JPG · WebP · AVIF · TIFF · Personalizar…
        Redimensionar  ▸ 25% · 50% · 1080p · 4K · Personalizar…
        Girar          ▸ 90° · 180° · 270°
        Espelhar       ▸ Horizontal · Vertical
        Ajustar cores  ▸ Personalizar…
        Visualizar em Íris
```

**13 formatos Tier-1 nativos** — PNG, JPG, WebP, **AVIF (default)**, TIFF, BMP, GIF, ICO, PNM, TGA, QOI, HDR, OpenEXR. HEIC, JPEG XL e RAW por trás de feature flags. Sem regressão para o legado útil que o IrfanView popularizou.

## Status — M1 testável

| Bloco | Estado |
|---|---|
| CLI: `convert` · `resize` · `rotate` · `flip` · `crop` · `adjust` · `remove-bg` · `upscale` · `install-integrations` | ✓ |
| Viewer GTK4: zoom cursor-anchored · drag pan · navegação · fullscreen | ✓ |
| Diálogos Prisma (modais): convert · resize · rotate · flip · adjust | ✓ |
| 6 gerenciadores de arquivos integrados | ✓ |
| IA local (BiRefNet-lite, MIT) — remove fundo via ONNX Runtime, com SHA-256 | ✓ |
| PKGBUILD principal (`xathay/bigiris`) e `.local` para build sem push | ✓ |
| Glycin sandboxed decode · film strip · EXIF sidebar · preview ao vivo | M2 |

## Remover fundo com IA — local, privado, sem conta

```bash
bigiris remove-bg foto.jpg              # CLI → foto_nobg.png (RGBA)
bigiris --dialog=remove-bg foto.jpg     # diálogo GUI
bigiris remove-bg *.jpg                 # lote
```

Na primeira execução, o modelo (~224 MB) é baixado uma única vez do [mirror oficial da comunidade ONNX](https://huggingface.co/onnx-community/BiRefNet_lite-ONNX) para `~/.local/share/iris/models/` e **verificado por SHA-256** contra o hash fixado no binário. Qualquer divergência aborta a instalação — um mirror comprometido não consegue injetar pesos alterados. Chamadas seguintes usam o cache local.

Documentação completa: [docs/IA-LOCAL.md](docs/IA-LOCAL.md) — por que isso importa, como funciona a verificação por hash, allowlist de licenças, e o que está planejado para upscale via IA (Real-ESRGAN, que hoje usa Lanczos3 em CPU).

## Como instalar

### Opção A — `makepkg` (Arch / BigLinux / Manjaro)

A partir do diretório `pkgbuild/`:

```bash
cd pkgbuild
makepkg -si                        # builda do remoto (git+https://github.com/xathay/bigiris)
makepkg -si -p PKGBUILD.local      # ou builda do checkout local, pula o clone
```

O hook `.install` informa que as integrações **já estão ativas system-wide** após a instalação — clique direito em qualquer imagem e o submenu "Íris ▸" aparece nos seis gerenciadores.

Para desinstalar: `sudo pacman -R bigiris`.

### Opção B — `cargo install` (qualquer distro)

```bash
# Dependências de sistema (Manjaro/BigLinux):
sudo pacman -S --needed gtk4 libadwaita dav1d hicolor-icon-theme rust pkgconf

# Da raiz do repo:
cargo install --path crates/bigiris --features gui --locked

# Garante PATH:
export PATH="$HOME/.cargo/bin:$PATH"

# Instala integrações de clique direito no seu usuário:
bigiris install-integrations --user
```

Para desinstalar: `bigiris uninstall-integrations && cargo uninstall bigiris` (rode o `uninstall-integrations` **antes** do `cargo uninstall`, senão o binário some).

### Opção C — `cargo run` (dev rápido, sem instalar)

```bash
cargo build --release --locked --features gui -p bigiris
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
#     Clique direito numa imagem → submenu Íris ▸
```

## CLI completa

```bash
# Transformações headless
bigiris convert --to png foto.jpg
bigiris convert --to avif --overwrite replace *.jpg
bigiris resize --max-edge 1080 foto.png
bigiris resize --percent 50 --filter lanczos3 foto.png
bigiris resize --exact 800x600 --to webp foto.png
bigiris resize --fit 3840x3840 foto.png          # upscale+downscale preservando aspecto
bigiris rotate --degrees 90 foto.jpg
bigiris flip --axis horizontal foto.jpg
bigiris crop --rect 800x600+100+50 foto.jpg
bigiris adjust --brightness 15 --contrast 10 foto.jpg
bigiris adjust --saturation -100 foto.jpg        # preto e branco
bigiris adjust --gamma 0.7 foto.jpg              # clareia midtones

# IA local (BiRefNet-lite MIT, offline)
bigiris remove-bg foto.jpg                       # → foto_nobg.png (RGBA)
bigiris remove-bg *.jpg                          # lote

# Upscale (Lanczos3 CPU hoje; Real-ESRGAN planejado)
bigiris upscale --factor 2 foto.jpg              # 2x, 3x ou 4x

# Diálogos modais Prisma (usados pelo "Personalizar…" dos menus)
bigiris --dialog=convert *.jpg
bigiris --dialog=resize *.png
bigiris --dialog=rotate foto.jpg
bigiris --dialog=flip foto.jpg
bigiris --dialog=adjust foto.jpg

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
│   ├── bigimage-core/          # Decode, encode, transforms — pure Rust, headless
│   ├── bigimage-ai/            # ORT + modelos (feature `ai`, lazy)
│   ├── bigimage-integrations/  # Service menus para 6 file managers
│   └── bigiris/                # Binário único (CLI + GUI via feature `gui`)
├── data/                       # .desktop, AppStream metainfo, gschema, ícone SVG
├── pkgbuild/                   # PKGBUILD + .install (Arch/BigLinux)
└── docs/                       # ADRs, IA-LOCAL
```

Decisões arquiteturais formalizadas em `docs/ADR-001`, `ADR-002`, `ADR-003`.

## Desenvolvimento

Requer Rust stable ≥ 1.83 e libs de sistema (`gtk4`, `libadwaita`, `dav1d`, `pkgconf`).

```bash
# Build sem GUI (CI headless, compila rápido)
cargo build

# Build com viewer GTK4 + diálogos
cargo build --features gui -p bigiris

# Qualidade
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p bigiris --features gui --all-targets -- -D warnings
cargo test --workspace
```

## Licença

GPL-3.0-or-later. Ver [LICENSE](LICENSE).

Parte da família **BigLinux**. Mantido por Leonardo Athayde.
