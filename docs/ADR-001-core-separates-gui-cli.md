# ADR-001 — Core separa GUI e CLI

- **Status:** Aceito
- **Data:** 2026-04-19

## Contexto

O prompt inicial exige (R3) que "as funcionalidades utilizadas pela interface gráfica também sejam acessíveis através da linha de comandos" para habilitar CI/CD que impeça regressões em recursos estabilizados. Sem essa separação, qualquer teste automatizado precisaria de display (Xvfb, Wayland headless) e introduziria flakiness.

## Decisão

Toda lógica de decode, encode, transforms, pipeline e IA mora em `bigimage-core` e `bigimage-ai` — **crates que NÃO dependem de `gtk4`, `adw` ou qualquer toolkit gráfico**. O binário `bigiris`:

- **Subcomandos CLI** (`bigiris convert …`, `bigiris resize …`, etc.) invocam o core diretamente, sem abrir janela.
- **GUI** (viewer + diálogos modais) é um cliente do core: captura parâmetros do usuário e chama a mesma API que o CLI.

Testes de integração disparam a MESMA operação pelas duas superfícies e comparam outputs byte-a-byte. Divergência quebra CI.

## Consequências

**Positivas**
- CI em contêiner Arch sem display valida 100% das regras de negócio.
- Usuários de terminal e scripts têm acesso nativo a tudo.
- Refactor do core não pode vazar para a GUI (barreira de compilação).

**Negativas**
- Parâmetros duplos: cada opção precisa ser exposta em clap (CLI) e no widget (GUI). Mitigado por geração parcial: structs de parâmetro serializáveis via serde, usadas por ambos.
- Tentação de colocar "só um detalhinho visual" no core. Revisões de PR devem recusar.

## Alternativas descartadas

- **Core dependendo de gtk4 para tipos de imagem**: acoplaria o core a um toolkit específico e inviabilizaria CI headless.
- **CLI como camada fina de bindings sobre GUI**: inverteria a ordem, GUI passaria a ser source-of-truth, testes dependeriam de display.
