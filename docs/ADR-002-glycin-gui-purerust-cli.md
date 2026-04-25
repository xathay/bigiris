# ADR-002 — glycin na GUI, pure-Rust no CLI

- **Status:** Aceito
- **Data:** 2026-04-19

## Contexto

Decodificar imagens em Linux envolve um zoológico de libs C com histórico denso de CVEs (libjpeg, libpng, libheif, libtiff…). A GUI precisa de decodificação segura, com suporte a formatos modernos (AVIF/HEIC/JXL/RAW) e sem travar a UI. O CLI precisa rodar em CI mínimo (contêiner Arch headless, possivelmente sem bwrap) e em instalações `makepkg` enxutas.

## Decisão

**Dois caminhos, um core:**

1. **GUI** usa [`glycin`](https://gitlab.gnome.org/sophie-h/glycin): cada loader de formato roda em processo separado em sandbox `bwrap` com syscalls restritas. Bug em libheif não compromete o app. É o backend do Loupe. Entrega AVIF/HEIC/JXL/RAW/HDR de graça.

2. **CLI** usa **pure-Rust** primeiro: `image` crate + `jxl-oxide` + `jpegxl-rs` (encode JXL) + `libheif-rs` (quando necessário) + `rawler` + `kamadak-exif`. Fallback opcional para **libvips** em batches gigantes (pipeline streaming, memória constante).

## Consequências

**Positivas**
- Segurança em profundidade no caminho GUI (R2).
- CI roda mesmo em contêiner sem bwrap (caminho pure-Rust).
- libvips opcional cobre o extremo de performance em batches enormes sem forçar toda compilação a depender dele.

**Negativas**
- Dois caminhos de decode significa duas APIs a manter em sincronia. Mitigado: ambos produzem o mesmo tipo `DecodedImage` do core.
- libheif e jpegxl-rs continuam dependendo de libs C no CLI. Aceitável porque rodam fora de sandbox apenas em contextos onde o usuário confia no input (CI, scripts próprios).

## Alternativas descartadas

- **Só glycin em todo lugar**: glycin é Linux-only e depende de bwrap. Não roda em CI mínimo nem em Alpine. Quebra requisito de CI headless.
- **Só pure-Rust em todo lugar**: perde cobertura de HEIC/RAW/HDR avançado que glycin já entrega.
- **ImageMagick/GraphicsMagick via CLI**: overhead de processo, superfície CVE grande, resultado menos controlável.
