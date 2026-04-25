# Política de Segurança

## Versões com suporte

Apenas a `main` e a tag mais recente recebem correções. Ainda não há
linha LTS — o projeto está pré-1.0 e a velocidade de lançamento é
semanal.

## Reportando uma vulnerabilidade

**Não abra issue pública nem PR para questões de segurança.** Use um
dos canais privados:

1. **GitHub Security Advisories** — em
   [github.com/xathay/bigiris/security/advisories/new](https://github.com/xathay/bigiris/security/advisories/new).
   Preferido: criptografa em trânsito e fica invisível até o disclosure.
2. **E-mail** — `leoathayde@gmail.com` com o assunto começando com
   `[BigIris-Security]`. Se quiser PGP, peça a chave pública por esse
   mesmo canal.

Inclua, na medida do possível:

- Versão do BigIris afetada (`bigiris --version`).
- Distro + ambiente gráfico (BigLinux/Manjaro/Arch/etc., GNOME/KDE/...).
- Reprodução mínima — se envolver arquivo malicioso, descreva sem anexar
  o exploit nesta mensagem; combine um canal seguro depois.
- Impacto observado (ex.: leitura de arquivos fora do escopo, execução
  de código, escalonamento de privilégio).

## Tempo de resposta

| Severidade                                    | Resposta inicial |
|-----------------------------------------------|------------------|
| Crítica (RCE, escalonamento de privilégio)    | até 48 h         |
| Alta (DoS local, leitura de arquivo arbitrária) | até 7 dias     |
| Média / Baixa                                 | até 30 dias      |

## Escopo

São considerados em escopo:

- Decode de arquivos de imagem (`bigimage-core`).
- Pipeline de IA local (`bigimage-ai`) — incluindo verificação de
  integridade dos modelos.
- Geradores de service menus de file manager (`bigimage-integrations`).
- Visualizador GTK4 (`bigiris/gui`).
- PKGBUILDs do projeto (`pkgbuild/`).

Fora de escopo (encaminhe upstream): vulnerabilidades em `image`,
`fast_image_resize`, `dav1d`, `gtk4`, `libadwaita`, `ort`, `glycin`,
ONNX Runtime ou no kernel.

## Hardening já aplicado

Para contexto sobre o que já é defesa em profundidade:

- `#![forbid(unsafe_code)]` em todos os crates.
- Decode com cap de bytes (1 GiB) e de pixels (256 MP) antes do
  decompress, refusando decode bombs e pixel bombs.
- Service menus escritos com `O_NOFOLLOW` para impedir
  symlink-follow durante `sudo install-integrations --system`.
- Modelos IA pinados por SHA-256 + allowlist FOSS de licenças
  (MIT/Apache-2.0/BSD/MPL-2.0/CC0); download abortado se exceder o
  tamanho esperado por mais de 16 MiB.
- Comandos do Thunar UCA empacotados em `sh -c '… "$@"' …` para que
  filenames não sejam reinterpretados pelo shell.
- TLS via `native-tls` (OpenSSL do sistema), sem `danger_*` desativando
  validação de certificado.
