# IA local no BigIris

> Todo o processamento de IA do BigIris roda **no seu computador**. Nada de
> upload, nada de "conta gratuita", nada de API externa. Assim que você
> instala o BigIris, a funcionalidade de **remover fundo** está pronta para
> ser usada — sem depender de serviços de terceiros.

## Por que isso importa

Soluções populares de remoção de fundo via web — `remove.bg`, `PhotoRoom`,
`Canva BG Remover`, integrações "IA" de editores online — têm três
problemas que o BigIris resolve por design:

1. **Privacidade.** Você envia a imagem para o servidor do fornecedor. O
   serviço pode registrar, indexar, treinar novos modelos, ou sofrer
   vazamento. Para quem trabalha com material sensível (documentos,
   identidade, registros médicos, provas em processos judiciais), isso é
   incompatível com dever de sigilo e com a LGPD.
2. **Dependência e risco operacional.** O serviço pode sumir, mudar de
   preço, introduzir limite de requisições, ou simplesmente ficar fora do
   ar no momento em que você precisa. O BigIris funciona offline, para
   sempre, com o mesmo resultado.
3. **Licenciamento opaco.** Muitos modelos de código aberto ("open weights")
   têm licença *non-commercial* escondida (ex.: `CC-BY-NC`, RAIL-M). O
   BigIris rejeita qualquer modelo fora de uma allowlist FOSS — veja a
   seção [Allowlist de licenças](#allowlist-de-licenças).

## O modelo que o BigIris usa: BiRefNet-lite

**BiRefNet** (*Bilateral Reference Network*) é uma arquitetura estado-da-arte
para segmentação dicotômica de imagem — isto é, separar "objeto" de "fundo"
em alta resolução, com bordas limpas em cabelo, folhagem, tecidos
translúcidos e outras regiões tipicamente difíceis.

- **Autor original:** Peng Zheng et al. (paper CAAI AIR'24).
- **Repositório canônico dos pesos:** [ZhengPeng7/BiRefNet_lite](https://huggingface.co/ZhengPeng7/BiRefNet_lite) no Hugging Face.
- **Variante usada pelo BigIris:** `birefnet-lite` — versão enxuta, 224 MB,
  FP32, otimizada para rodar em CPU doméstica em poucos segundos por imagem.
- **Licença:** **MIT** (uma das licenças FOSS mais permissivas — uso
  pessoal, comercial, redistribuição livres, sem cláusula viral).
- **Distribuição ONNX:** O BigIris consome o mirror oficial da comunidade
  [onnx-community/BiRefNet_lite-ONNX](https://huggingface.co/onnx-community/BiRefNet_lite-ONNX),
  mantido pela equipe Xenova/Hugging Face — que exporta os pesos
  originais para o formato ONNX padronizado. Licença **MIT** herdada.

### Como verificamos a integridade do modelo

A URL e o hash SHA-256 do arquivo ficam **congelados em tempo de
compilação** em `crates/bigimage-ai/src/background.rs`:

```rust
pub const BIREFNET_LITE: ModelSource = ModelSource {
    id: "birefnet-lite",
    url: "https://huggingface.co/onnx-community/BiRefNet_lite-ONNX/resolve/main/onnx/model.onnx",
    license_spdx: "MIT",
    sha256: "5600024376f572a557870a5eb0afb1e5961636bef4e1e22132025467d0f03333",
    size_bytes: 224_005_088,
    description: "BiRefNet-lite — remoção de fundo FP32, MIT (onnx-community mirror).",
};
```

Na primeira vez que você pede uma remoção de fundo, o BigIris:

1. Verifica se a licença (`MIT`) está na [allowlist FOSS](#allowlist-de-licenças);
2. Baixa o arquivo para `~/.local/share/iris/models/birefnet-lite.onnx.part`;
3. Calcula o SHA-256 e compara com o hash fixado no binário;
4. Se bater, renomeia para `birefnet-lite.onnx`. Se não bater, **apaga**
   o parcial e aborta com erro — nenhum byte desconhecido é aceito.

Dali em diante, qualquer chamada reutiliza o modelo em cache. Se o arquivo
em disco for corrompido ou substituído, o hash falha e o BigIris baixa
de novo. Um mirror comprometido **não** consegue injetar outro modelo.

## Allowlist de licenças

Código em `crates/bigimage-ai/src/download.rs`:

```rust
pub fn allowed_licenses() -> &'static [&'static str] {
    &["MIT", "Apache-2.0", "BSD-3-Clause", "BSD-2-Clause", "MPL-2.0", "CC0-1.0"]
}
```

Apenas modelos cujo SPDX esteja nessa lista são aceitos. Tentativas de
carregar pesos com licença `CC-BY-NC-4.0`, `RAIL-M`, "other", "Qualcomm
AI Hub Terms" e similares falham com `LicenseNotAllowed(...)` **antes** de
qualquer download. Isso é um guard-rail deliberado contra supply-chain
FOSS-washed — muitos modelos publicam pesos rotulados como "open" mas com
cláusulas que proíbem uso comercial ou redistribuição.

Se você quiser incluir um modelo com licença fora da lista, a mudança é
intencional: editar o código, recompilar, documentar por quê.

## Como usar

Depois de instalar o BigIris (`pacman -S bigiris` ou equivalente com
`--features ai`):

### CLI headless
```bash
bigiris remove-bg foto.jpg
# Saída: foto_nobg.png (PNG com canal alfa)
```

### Diálogo GUI
```bash
bigiris --dialog=remove-bg foto.jpg
```

### Dentro do visualizador
Abra a imagem no BigIris e use o menu **Íris ▸ Remover fundo** — mesmo
backend, mesmo resultado, sem linha de comando.

Primeira execução: ~10 s de download (224 MB, uma única vez). A partir daí,
cada imagem leva alguns segundos em CPU de desktop comum.

### O que acontece quando você clica em "Remover fundo"

1. **Barra de progresso ao vivo.** O diálogo monta uma barra de progresso
   com duas fases distintas:
   - *"Baixando modelo (uma vez só): 42 MB / 224 MB"* — só na primeira
     execução; nas próximas fica invisível porque o cache é hash-verificado.
   - *"Processando 1/1 — foto.jpg"* — inferência em andamento no arquivo
     atual; em lote, o contador avança a cada imagem.
2. **Trabalho sai da thread da UI.** A inferência roda numa *worker thread*
   dedicada, então a janela continua responsiva — você pode mover,
   minimizar, ler a barra de progresso sem travar.
3. **Antes × depois, lado a lado.** Ao terminar o processamento de uma
   única imagem, o BigIris abre automaticamente o **visualizador de
   comparação** (`--dialog=compare`) com a imagem original à esquerda e
   o resultado com fundo transparente à direita. Em lote (múltiplos
   arquivos), o diálogo mostra só o sumário — você abre cada comparação
   manualmente pelo gerenciador de arquivos.

## Requisitos de sistema

- **`onnxruntime`** instalado no sistema (Arch/Manjaro: `sudo pacman -S onnxruntime`).
  O pacote oficial do BigLinux já declara essa dependência.
- **Espaço em disco:** ~225 MB para o modelo em cache.
- **RAM:** ~1 GB livre durante a inferência de uma imagem típica.
- **GPU:** opcional. O BigIris roda na CPU por padrão — GPU-accelerated
  (CUDA/ROCm) é configuração futura.

Caso você tenha instalado uma build sem a feature `ai` (ex.: compilou
manualmente sem a flag), o diálogo mostra um banner explicativo pedindo
para recompilar com `cargo build --features ai`, sem travar o app.

## Modelo adicional previsto: Real-ESRGAN

A segunda frente de IA é **upscale** (aumentar resolução com rede
neural). Na versão atual, o BigIris usa **Lanczos3 em CPU** — um
reamostrador clássico, muito bom para 2×/3×/4×, que roda instantaneamente
e não precisa de modelo.

O backend IA (Real-ESRGAN) está planejado — infra de download já pronta
— mas requer *tiled inference* (dividir a imagem em janelas de 128×128,
inferir cada uma, costurar de volta com overlap). É trabalho de
engenharia já escopado; entra numa iteração futura. Candidatos FOSS
levantados:

- [bukuroo/RealESRGAN-ONNX](https://huggingface.co/bukuroo/RealESRGAN-ONNX) — BSD-3-Clause, x4plus, 67 MB.
- [tidus2102/Real-ESRGAN](https://huggingface.co/tidus2102/Real-ESRGAN) — BSD-3-Clause, x2plus, 67 MB.

Até lá, Lanczos3 resolve o caso comum de duplicar/triplicar tamanho.

## Resumo para o usuário final

- ✔ **Local, offline, privado** — a imagem nunca sai do seu computador.
- ✔ **FOSS pura** — BiRefNet-lite sob licença MIT, distribuído pelo mirror oficial da comunidade ONNX.
- ✔ **Verificação por hash** — BigIris rejeita qualquer arquivo que não bata com o hash fixado no binário.
- ✔ **Zero conta, zero fila, zero limite mensal** — uma instalação, uso ilimitado.
- ✔ **Integrado** — funciona no viewer, em diálogo modal e em linha de comando.

Dúvidas sobre licenças, modelos ou a mecânica de verificação? Abra um
issue no repositório — ou leia diretamente o código: é pequeno, é
auditável, é esse o ponto.
