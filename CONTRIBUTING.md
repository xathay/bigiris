# Contribuindo

BigIris é Rust + GTK4/libadwaita, single-binary com módulos `Íris`
(viewer) e `Prisma` (transformações). Esse documento cobre o
necessário pra você compilar, testar, traduzir e enviar mudanças
sem fricção.

## Pré-requisitos

Distros baseadas em Arch/BigLinux:

```bash
sudo pacman -S --needed rust pkgconf gtk4 libadwaita dav1d openssl gettext
```

Distros Debian/Ubuntu:

```bash
sudo apt-get install -y rustc cargo pkg-config libgtk-4-dev \
    libadwaita-1-dev libdav1d-dev libssl-dev gettext
```

Rust mínimo: **1.83** (definido em `rust-toolchain.toml`; `rustup`
respeita automaticamente).

## Compilando

```bash
# Build sem GUI (rápido, headless — o que CI roda):
cargo build

# Build completo (viewer + diálogos + IA):
cargo build --features "gui ai" -p bigiris

# Release:
cargo build --release --features "gui ai" -p bigiris
```

## Testando

```bash
# Workspace inteiro (headless):
cargo test --workspace --all-targets --locked

# GUI + IA (precisa das libs de sistema acima):
cargo test -p bigiris --features "gui ai" --all-targets --locked

# Self-test do binário:
./target/release/bigiris --self-test
```

## Qualidade — antes de cada commit

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p bigiris --features "gui ai" --all-targets --locked -- -D warnings
```

Os 3 são gates obrigatórios no CI (`.github/workflows/ci.yml`).
Warnings viram erro, então PRs com clippy sujo não passam.

## Estrutura

```
crates/
├── bigimage-core/         # decode/encode/transforms — pure Rust, headless
├── bigimage-ai/           # ONNX Runtime + modelos (feature `onnx`)
├── bigimage-integrations/ # service menus pros 6 file managers
└── bigiris/               # binário (CLI + GUI via feature `gui`)
data/
├── com.biglinux.Iris.{desktop,metainfo.xml,gschema.xml}
├── icons/hicolor/scalable/apps/com.biglinux.Iris.svg
├── nautilus/bigiris-menu.py
└── po/                    # catálogos i18n
docs/
└── ADR-00{1,2,3}-*.md     # decisões arquiteturais
pkgbuild/
├── PKGBUILD               # build do remoto
└── PKGBUILD.local         # build do checkout
```

ADRs explicam *por que* o código está assim — leia antes de propor
refatoração estrutural.

## i18n

Strings de UI ficam em `data/po/`. O fluxo:

```bash
# Após adicionar/alterar gettext("…") em código fonte:
data/po/regen-pot.sh

# Edite data/po/<lang>.po pra preencher os msgstr novos.
# msgfmt valida antes de comitar:
msgfmt --check data/po/en.po -o /dev/null
```

A linguagem-fonte é **pt-BR**. Catálogos (`en.po`, futuros `es.po`,
etc.) traduzem a partir do pt-BR.

## Convenções

- **Idioma**: pt-BR para conteúdo de usuário (UI, mensagens, docs);
  inglês para código, identificadores e mensagens de commit.
- **Commits**: estilo Conventional Commits sem ser dogmático —
  `feat:`, `fix:`, `docs:`, `ci:`, `refactor:`, `security:`,
  `i18n:`, `deps:`. Mensagens explicam *por quê*, não *o quê* (o
  diff já mostra o quê).
- **Comentários**: só quando o "por quê" não é óbvio do código.
  Não documente o que um nome bem escolhido já comunica.
- **Errors**: `bigimage-core` define `BigImageError`; binário
  converte para `color_eyre::Result` na borda.
- **No `unsafe`**: todos os crates têm `#![forbid(unsafe_code)]`.
- **GPL-3.0-or-later**: todo arquivo `.rs`/`.py` começa com
  `// SPDX-License-Identifier: GPL-3.0-or-later`.

## Fluxo de PR

1. Abra issue antes pra mudanças não-triviais — alinha escopo.
2. Branch a partir de `main`.
3. Commits pequenos, mensagens informativas.
4. Garanta `cargo fmt --check`, ambos `clippy -D warnings`, e `cargo
   test` passando antes de pedir review.
5. Se a mudança afeta a UI ou o pacote, descreva como testou
   localmente (a CI valida sintaxe, não UX).

## Reportando vulnerabilidades

Não abra PR/issue pública. Veja [SECURITY.md](SECURITY.md).
