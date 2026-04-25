# BigIris M2 — Design de UX premium

**Status:** Proposta, ainda não implementada. Discute o salto qualitativo do M1 (esqueleto funcional) para um produto que se diferencia do ReImage clássico (KDE / bash+kdialog) e dos viewers atuais.

**Pressupostos** (Leonardo, 2026-04-20):
1. Não copiar ReImage — absorver o que é útil, reinventar o resto.
2. UI moderna, alinhada ao stack 2026: GTK4 + libadwaita, Blueprint, preview GPU ao vivo.
3. Surpreender: cada operação tem algo que o ReImage não tem.

---

## 1. Inventário — o que o ReImage BigLinux expõe hoje

Fonte: [github.com/biglinux/kde-service-menu-reimage](https://github.com/biglinux/kde-service-menu-reimage) (versão 26.01.04, bash + kdialog).

### 1.1 Grupo "Comprimir e redimensionar"

| ReImage | Estado no BigIris M1 |
|---|---|
| `progressive` (JPEG progressivo) | ✗ não temos |
| `optimize` (otimização web: strip metadata, JPEG progressive, PNG optipng-like) | ✗ não temos |
| `compress_50/70/75/80/90` (JPEG quality presets) | ✗ não temos (nosso convert não expõe quality) |
| `custom_compress` (diálogo de qualidade) | ✗ |
| `resize_25/33/50/67/75` (percent presets) | ✓ (25%, 50%; faltam 33/67/75) |
| `320x240` `800x600` `1024x768` `1200x900` `1400x1050` `1920x1080` `1920x1200` `2560x1080` `2560x1440` `2560x1600` `3440x1440` `3840x2160` `5120x2160` (13 pixel presets) | ✓ parcial (apenas HD 1920 e 4K 3840; faltam os outros) |
| `custom_resize` | ✓ (nosso diálogo cobre 4 modos; kdialog deles é apenas inputbox) |

### 1.2 Grupo "Converter e girar"

| ReImage | Estado M1 |
|---|---|
| `avif` `heic` `jpeg` `jxl` `png` `gif` `tiff` `webp` | ✓ (menos heic/jxl — Tier-2 reservado) |
| `pdf` `pdfa` (PDF/A-1) | ✗ |
| `favicons` (gera set `.ico` + tamanhos) | ✗ |
| `base64` (encode data-URI) | ✗ |
| `convert_custom` / `formats` (lista) | ✓ diálogo |
| `rotate_90/180/270` `rotate_custom` | ✓ (menos custom — só cardinais) |
| `auto` (rotate via EXIF) | ✗ |
| `flip` `flop` (H/V) | ✓ |

### 1.3 Grupo "Metadata"

| ReImage | Estado M1 |
|---|---|
| `rfe`/`rff` (rename file from EXIF / filename) | ✗ |
| `sffe`/`sffn` (set filetime from EXIF / name) | ✗ |
| `seff`/`sefn` (set EXIF from filetime / name) | ✗ |
| `add_comment` / `view_metadata` / `extract_metadata` | ✗ |
| `del_comment` `del_exif` `del_iptc` `del_xmp` `del_all` | ✗ (planejado em PLAN §10 "strip GPS em destaque") |
| `tfe` (transfer EXIF) | ✗ |

### 1.4 Grupo "Utilitários"

| ReImage | Estado M1 |
|---|---|
| `agif` `apng` `webp` (animated) | ✗ |
| `append_right` (compose montage) | ✗ |
| `gray` (greyscale) | ✓ (via adjust --saturation -100) |
| `sepia` | ✗ (direto — faltam filtros nomeados) |
| `transparent2color` (alpha → cor) | ✗ |
| `border` `border_transparent` `shadow` | ✗ |

### 1.5 Wallpaper

| ReImage | Estado M1 |
|---|---|
| `SetAsWallpaperAndLockScreen` | ✗ (mas FMs nativos já oferecem) |

---

## 2. Diferencial técnico do BigIris (o que NENHUM ReImage tem)

### 2.1 Preview ao vivo em todo diálogo

Arquitetura reutiliza o canvas GPU do viewer Íris (PLAN §6.3):

```
AdwDialog
├── AdwHeaderBar
├── [controles / sliders / combos]   ←─┐
│                                       │  eventos de mudança
│   (debounced ~80ms)                   │
│                                       ↓
├── gtk::Picture (preview)   ←── pipeline aplica op em thumbnail 600px
└── [botões Cancelar / Aplicar]
```

- Slider de brilho → thumbnail atualiza em < 100ms.
- ComboRow de formato → preview mostra diferença visual (AVIF de 80% vs PNG vs JPEG de 70%) — o usuário compara ANTES de gravar.
- Compara antes/depois via gesture ou botão de toggle.

### 2.2 Prompts inteligentes baseados no arquivo

- Arquivo é PNG com transparência → dialog de "Converter para JPG" avisa que vai perder canal alpha e sugere manter PNG ou migrar pra WebP.
- Arquivo tem EXIF GPS → banner no topo: "Esta imagem contém sua localização. Remover antes de publicar?"
- Imagem tem 20MP → dialog de resize sugere "1920×1080 (economizaria 95% do tamanho)".

### 2.3 Batch com percentual crescente

Service-menu do ReImage passa files via argv e trava o Dolphin até terminar. BigIris:
- Janela de progresso não-modal que segue em cascata mesmo se o FM fechar.
- Usa `tokio` + canal `mpsc` → redraw a cada imagem.
- Botão "cancelar a meio" funciona (propagação via `CancellationToken`).

### 2.4 Undo histórico e sidecar

- Cada operação grava sidecar `.bigiris.json` ao lado do original com o pipeline usado.
- "Refazer" abre o diálogo pré-populado com os parâmetros da última vez.
- "Desfazer em massa" (viewer ou diálogo): volta os 10 últimos arquivos tocados.

### 2.5 Ações que não existem no ReImage (ideias modernas)

| Idéia | Observação |
|---|---|
| **Background-remove em 1 clique** (BiRefNet) | PLAN §7 já prevê |
| **Upscale IA 2×/4×** (Real-ESRGAN) | PLAN §7 |
| **Denoise** (SCUNet) | PLAN §5.1 M3 |
| **Extração OCR** (PaddleOCR) | PLAN §5.1 M3 — ReImage só viu como "Reconhecimento de texto OCR" via outro pacote |
| **"Preparar pra WhatsApp"** | Preset: max-edge 1280 + JPEG q85 + strip GPS + progressive. 1 clique. |
| **"Preparar pra impressão A4 300dpi"** | Preset inteligente sabendo o tamanho físico |
| **"Comparar dois"** | Selecionar 2 imagens → slider wipe no viewer (antes/depois, diff pixel-a-pixel, SSIM numeric) |
| **"Variar formato"** | Preview grid mostrando o MESMO conteúdo em PNG / JPG 85% / WebP / AVIF com tamanhos ao lado — usuário escolhe visualmente |
| **"QR code desta imagem"** | Gera link `data:` QR scannable (aproveita nosso base64 implícito) |

---

## 3. Proposta de nova matriz de ações (M2)

Conservando a hierarquia BigIris ▸ mas expandida:

```
Íris ▸
├── Para web / redes sociais ▸       ← NOVO agrupamento por uso
│   ├── WhatsApp (1280px, q85, strip GPS)
│   ├── Instagram (1080px square, q90)
│   ├── Facebook (2048px, q85)
│   ├── Twitter/X (1200×675, q85)
│   └── Otimizar (mantém formato, reduz tamanho)
├── Converter ▸
│   ├── [preview do resultado]
│   ├── PNG · JPG · WebP · AVIF · TIFF · GIF · HEIC · JPEG XL
│   ├── PDF · PDF/A-1                     ← NOVO
│   ├── Favicon (multi-size .ico)         ← NOVO
│   ├── Data URI (base64)                 ← NOVO
│   └── Personalizar…
├── Redimensionar ▸
│   ├── Presets: 25% · 33% · 50% · 67% · 75% · 200%
│   ├── HD (1920px) · 2K (2560px) · 4K (3840px) · 5K (5120px)
│   ├── Preparar pra impressão A4 300dpi  ← NOVO (inteligente)
│   └── Personalizar…
├── Girar e espelhar ▸                  ← fundido (ReImage agrupa assim)
│   ├── 90° · 180° · 270°
│   ├── Automático (EXIF orientation)    ← NOVO
│   ├── Espelhar H · Espelhar V
│   └── Ângulo personalizado             ← imageproc futuro
├── Ajustar ▸
│   ├── [preview ao vivo com sliders]
│   ├── +Brilho · −Brilho
│   ├── +Contraste · −Contraste
│   ├── Vívido · Desbotar · P&B · Sépia   ← filtros nomeados
│   └── Personalizar…
├── IA ▸                                ← NOVO (M2)
│   ├── Remover fundo
│   ├── Upscale 2×
│   ├── Upscale 4×
│   └── Denoise
├── Metadados ▸                         ← NOVO (M2)
│   ├── Ver tudo (sidebar no viewer)
│   ├── Remover localização (GPS)        ← LGPD highlight
│   ├── Remover tudo (EXIF + IPTC + XMP)
│   └── Renomear por data EXIF
├── Utilidades ▸                        ← NOVO agrupamento
│   ├── Criar GIF/WebP/APNG animado
│   ├── Juntar imagens lado-a-lado
│   ├── Adicionar borda / sombra
│   ├── Marca d'água
│   └── QR desta imagem
└── Visualizar em Íris
```

11 submenus vs 6 atuais. Cada "Personalizar…" abre diálogo com preview ao vivo.

---

## 4. Refatoração técnica necessária

1. **Preview pipeline** — `bigimage-core::preview(ops, thumb_size=600) -> DynamicImage` que aplica qualquer combinação de transformações com debounce. Reutilizar lookup-tables (gamma LUT) entre invocações.
2. **Undo sidecar** — schema JSON versionado `.bigiris.json` + core API `undo(original, sidecar) -> Result<()>`.
3. **Quality/compression** — expandir `Format` enum para carregar `{ quality: Option<u8>, progressive: bool, optimize: bool }` no convert pipeline. Usar `mozjpeg` (via `mozjpeg-sys` crate) para compressão top-tier.
4. **Smart prompts** — `bigimage-core::analyze(input) -> AnalysisHint` detecta alpha, EXIF GPS, resolution, format. Dialogs leem o hint e adaptam.
5. **Batch não-modal** — janela separada via segunda `AdwApplicationWindow`, sobrevive ao fechamento do service-menu caller.
6. **Presets TOML** — `data/presets/whatsapp.toml`, `instagram.toml`, etc. Usuário pode editar `~/.config/bigiris/presets/*.toml`.

---

## 5. Roadmap implementacional (M2)

Ordem de impacto vs esforço:

| # | Feature | Impacto | Esforço | Bloqueio |
|---|---|---|---|---|
| 1 | Preview ao vivo em diálogos existentes (convert/resize/adjust) | ⭐⭐⭐ alto | médio | nenhum |
| 2 | Quality/progressive/optimize no convert | ⭐⭐⭐ | baixo | avaliar `mozjpeg` |
| 3 | Smart prompts (alpha/GPS/resolution) | ⭐⭐ | baixo | depende de #1 |
| 4 | Presets "Para web / redes" | ⭐⭐⭐ | baixo | depende de #2 |
| 5 | Metadata (view/strip GPS/strip all) | ⭐⭐ | médio | `kamadak-exif` |
| 6 | Rotate auto via EXIF | ⭐⭐ | baixo | depende de #5 |
| 7 | Preview grid "Variar formato" | ⭐⭐ diferencial | médio | depende de #1 |
| 8 | Batch não-modal com cancelamento | ⭐⭐ | médio | `tokio` + `CancellationToken` |
| 9 | Undo sidecar | ⭐ | médio | nenhum |
| 10 | IA (remove-bg, upscale) | ⭐⭐⭐ wow | alto | `ort` + download de modelos |
| 11 | Favicon / data URI / PDF | ⭐ | baixo | `resvg`/`lopdf` |
| 12 | Animated GIF/WebP | ⭐ | médio | `image` animated feature |
| 13 | Comparar dois (wipe slider) | ⭐⭐ | médio | viewer extension |
| 14 | Filtros nomeados (sépia, desbotar) | ⭐ | baixo | depende de adjust |
| 15 | Marca d'água | ⭐ | médio | adiciona operação |

**Sugestão de primeiro sprint M2:** #1 + #2 + #3 + #4 — em sequência, dá pra entregar "Preview ao vivo + qualidade JPEG controlável + smart prompts + presets sociais" numa iteração. Esse combo já transforma a sensação do produto.

---

## 6. Princípios de UX inegociáveis

- **Preview é obrigatório** em qualquer diálogo com >1 parâmetro configurável.
- **Reduzir cliques** — se uma ação tem apenas 1 parâmetro óbvio, executar direto e dar undo, em vez de abrir diálogo.
- **Respeitar o tempo** — cancelar é sempre um botão visível, não um atalho escondido.
- **Mostrar o tamanho** — toda operação indica "de 4.2MB → 890KB" antes de aplicar.
- **Respeitar o original** — nunca sobrescrever sem opt-in explícito (já implementado via `OverwritePolicy::Skip` default).
