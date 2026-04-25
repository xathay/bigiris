# BigIris

**Visualizador e conversor de imagens moderno para Linux**, escrito em Rust + GTK4/libadwaita. Parte da família BigLinux.

Substituto moderno do ServiceMenu *ReImage* (KDE) unificado com um visualizador no nível de Loupe/Gwenview: um único app com módulos **Íris** (visualizar) e **Prisma** (converter, redimensionar, girar, cortar, IA).

## Objetivos

- **Premium FOSS** — apto a disputar concursos internacionais de UI/UX.
- **Seguro** — decodificação sandboxed por formato (glycin), sem confiança em libs C.
- **Otimizado** — SIMD, GPU-accelerated render, memória constante em batches gigantes.
- **Testável** — toda funcionalidade espelhada em CLI (`bigiris convert ...`) para CI/CD.
- **Amplo leque de formatos** — inspiração IrfanView: modernos (AVIF já default;
  HEIC/JXL/RAW atrás de feature flags) + legado útil (PNM, TGA, HDR, OpenEXR).
- **Integrado** — clique direito funciona em Dolphin, Nautilus, Nemo, Thunar, PCManFM-Qt, elementary Files.
- **IA local** — remover fundo com **BiRefNet-lite** (MIT) rodando 100% no seu computador. Nada de upload, nada de conta, nada de API externa. Detalhes em [docs/IA-LOCAL.md](docs/IA-LOCAL.md).

## Status — M1 testável

- **CLI completo** — convert · resize · rotate · flip · crop · **adjust** (brilho/contraste/saturação/gamma) · install-integrations.
- **13 formatos Tier-1** — PNG, JPG, WebP, AVIF, TIFF, BMP, GIF, ICO, PNM, TGA, QOI, HDR, OpenEXR.
- **Viewer GTK4** — zoom cursor-anchored, drag pan em qualquer botão, navegação entre arquivos.
- **5 diálogos modais Prisma** — convert, resize, rotate, flip, adjust.
- **6 gerenciadores de arquivos** — Dolphin · Nautilus (extensão Python top-level) · Nemo · Thunar (merge uca.xml) · PCManFM-Qt · elementary Files.
- **PKGBUILD** — principal e `.local` para build sem push.

Menu completo no clique direito: **Íris ▸** com Converter · Redimensionar · Girar · Espelhar · Ajustar cores · Visualizar em Íris — cada subgrupo com "Personalizar…" que abre o respectivo diálogo Prisma.

Ver [PLAN.md](PLAN.md) para o roadmap completo (M2 traz IA, film strip, EXIF, preview ao vivo).

## Remover fundo com IA — local, privado, sem conta

Desde a rodada M2+, o BigIris remove fundo de imagens usando o modelo **BiRefNet-lite** (licença **MIT**), baixado uma única vez do [mirror oficial da comunidade ONNX](https://huggingface.co/onnx-community/BiRefNet_lite-ONNX) e **executado 100% no seu computador**. A imagem nunca sai do disco — não há upload, não há serviço externo, não há limite de créditos.

Na primeira execução, o arquivo (~224 MB) é baixado para `~/.local/share/iris/models/` e **verificado por SHA-256** contra o hash fixado no binário. Qualquer divergência aborta a instalação do modelo — um mirror comprometido não consegue injetar pesos alterados. Chamadas seguintes usam o cache local.

```bash
bigiris remove-bg foto.jpg              # CLI → foto_nobg.png (RGBA)
bigiris --dialog=remove-bg foto.jpg     # diálogo GUI
```

O backend só aceita modelos com licença **MIT / Apache-2.0 / BSD / MPL-2.0 / CC0** (allowlist FOSS rígida em `crates/bigimage-ai/src/download.rs`). Pesos "open" com cláusula *non-commercial* escondida são recusados antes mesmo do download.

Documentação completa: [docs/IA-LOCAL.md](docs/IA-LOCAL.md) — por que isso importa, como funciona a verificação por hash, allowlist de licenças, e o que está planejado para upscale via IA (Real-ESRGAN, que hoje usa Lanczos3 em CPU).

## Como testar hoje

O PKGBUILD principal aponta para `github.com/xathay/bigiris`. Para testar mudanças locais antes do push, use o **caminho local** descrito na Opção A abaixo.

### Opção A — `makepkg` local (pacote pacman-gerenciado)

A partir do diretório `pkgbuild/`:

```bash
cd pkgbuild
makepkg -si -p PKGBUILD.local      # builda do checkout local, pula o clone remoto
```

O `.install` hook informa que as integrações **já estão ativas system-wide** — clique direito em qualquer imagem e o submenu "Íris ▸" aparece em Dolphin, Nautilus (via scripts), Nemo, Thunar, PCManFM-Qt e elementary Files.

Para desinstalar o pacote depois: `sudo pacman -R bigiris`.

### Opção B — `cargo install` (instala em `~/.cargo/bin`)

**A partir da raiz do repositório** (não do `pkgbuild/`):

```bash
# Dependências do sistema (Manjaro/BigLinux):
sudo pacman -S --needed gtk4 libadwaita dav1d hicolor-icon-theme rust pkgconf

# Da raiz do repo:
cargo install --path crates/bigiris --features gui --locked

# Adiciona ~/.cargo/bin ao PATH se ainda não estiver:
export PATH="$HOME/.cargo/bin:$PATH"

# Instala as integrações de clique direito no seu usuário:
bigiris install-integrations --user
```

Para desinstalar depois: `cargo uninstall bigiris && bigiris uninstall-integrations` (rode o uninstall **antes** do cargo uninstall, senão o binário já terá sumido).

### Opção C — `cargo run` (sem instalar, para dev rápido)

```bash
# Da raiz do repo:
cargo build --release --locked --features gui -p bigiris
./target/release/bigiris foto.jpg             # viewer
./target/release/bigiris --dialog=convert foto.jpg  # dialog Prisma
```

### 3. Roteiro de teste (5 minutos)

```bash
# (a) Self-test — valida o binário
bigiris --self-test

# (b) CLI puro — crie uma imagem qualquer
bigiris convert --to avif minha-foto.jpg
bigiris resize --max-edge 1080 minha-foto.jpg
bigiris rotate --degrees 90 minha-foto.jpg
bigiris flip --axis horizontal minha-foto.jpg
bigiris crop --rect 800x600+100+50 minha-foto.jpg

# (c) Viewer — abrir um ou mais arquivos
bigiris foto.jpg *.png
#   Teclado: +/- (zoom), 0 (ajustar), 1 (1:1), ←/→ (prev/next),
#            Home/End, Space/Backspace, F11 (fullscreen), Esc
#   Mouse:   scroll wheel = zoom centrado no cursor
#            drag (qualquer botão) = pan quando zoomed in

# (d) Dialog modals (Prisma) — acessíveis pelo submenu "Íris ▸ X ▸ Personalizar…"
bigiris --dialog=convert foto.jpg
bigiris --dialog=resize foto.jpg

# (e) Integração com file manager — clique direito numa imagem
#   Menu Íris ▸ aparece com:
#     Converter ▸ PNG · JPG · WebP · AVIF · TIFF · Personalizar…
#     Redimensionar ▸ 25% · 50% · 1080p · 4K · Personalizar…
#     Girar ▸ 90° · 180° · 270°
#     Espelhar ▸ Horizontal · Vertical
#     Visualizar em Íris
```

### 4. Desfazer (remover integrações do seu usuário)

```bash
bigiris uninstall-integrations
```

O pacote em si sai com `pacman -R bigiris` (remove instalações system-wide e as pessoais quando ainda presentes).

## Estrutura do repositório

```
bigiris/
├── crates/
│   ├── bigimage-core/          # Lógica pura (decode, encode, transforms) — headless
│   ├── bigimage-ai/             # ORT + modelos (feature-gated, M2)
│   ├── bigimage-integrations/   # Service menus: Dolphin · Nautilus · Nemo · Thunar · PCManFM-Qt · elementary
│   └── bigiris/                 # Binário (GUI + CLI)
├── data/                        # .desktop, metainfo AppStream, gschema, ícone SVG
├── pkgbuild/                    # PKGBUILD + .install para Arch/BigLinux
└── docs/                        # ADRs
```

## CLI completo

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

# IA local — remover fundo (BiRefNet-lite MIT, offline)
bigiris remove-bg foto.jpg                        # → foto_nobg.png (RGBA)
bigiris remove-bg *.jpg                           # lote

# Upscale (Lanczos3 CPU hoje; Real-ESRGAN planejado)
bigiris upscale --factor 2 foto.jpg               # 2x, 3x ou 4x

# Diálogos modais Prisma (usados pelo "Personalizar…" dos menus)
bigiris --dialog=convert *.jpg
bigiris --dialog=resize *.png
bigiris --dialog=rotate foto.jpg
bigiris --dialog=flip foto.jpg
bigiris --dialog=adjust foto.jpg

# Viewer
bigiris foto.jpg                    # janela com imagem
bigiris                             # janela vazia
bigiris *.png                       # galeria (← → para navegar)

# Integrações
bigiris install-integrations --user
bigiris install-integrations --system --destdir=/tmp/stage   # para packaging
bigiris uninstall-integrations

# Debug
bigiris --self-test
bigiris --version
```

## Desenvolvimento

Requer Rust stable (≥ 1.83) e as libs de sistema (`gtk4`, `libadwaita`, `dav1d`).

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
