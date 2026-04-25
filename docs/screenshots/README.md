# Screenshots

Imagens referenciadas pelo `<screenshots>` em
[`data/com.biglinux.Iris.metainfo.xml`](../../data/com.biglinux.Iris.metainfo.xml).
Flathub e KDE Discover puxam essas URLs (resolvidas via raw.githubusercontent)
para a página do aplicativo, então mexer aqui muda o que o usuário vê na loja.

## Contrato

- **Resolução**: 1280×720 ou maior, até 3840×2160. Razão livre, mas
  AppStream prefere `~16:9` para o screenshot principal.
- **Formato**: PNG sem perfil ICC embutido (`magick out.png` já satisfaz).
- **Nome**: `NN-descricao.png` em ordem do que aparece primeiro. O
  primeiro vira `<screenshot type="default">` no metainfo.
- **Conteúdo**: real-app, sem watermarks, sem mock-ups grosseiros.

## Estado atual

`01-viewer.png` é um **placeholder** gerado via ImageMagick (logo +
gradient + texto). Substituir antes do repo virar público:

```bash
# Captura uma janela específica do BigIris no GNOME:
gnome-screenshot --window --file=docs/screenshots/01-viewer.png

# Ou no KDE com Spectacle:
spectacle --activewindow --background --output=docs/screenshots/01-viewer.png
```

Sequência sugerida pra publicação:

1. `01-viewer.png` — visualizador com uma foto carregada, header bar visível
2. `02-prisma-convert.png` — diálogo Convert (Prisma) com preview
3. `03-prisma-resize.png` — diálogo Resize com modos
4. `04-remove-bg.png` — antes/depois de remove-bg (IA local)
5. `05-file-manager.png` — submenu Íris ▸ no clique direito do Dolphin

Cada PNG vira uma `<screenshot>` no metainfo, na ordem.

## Validação

```bash
appstreamcli validate --explain data/com.biglinux.Iris.metainfo.xml
```

Falha se `width`/`height` declarados não baterem com o arquivo real;
relaxa a validação `--no-net` enquanto o repo é privado.
