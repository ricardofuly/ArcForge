# Revisão de segurança do ArcForge

Data: 02/10/2026. Código analisado: commit `90f9232cb368a3c011b263831985ce2f0f6fad8e`.

## Estado após as correções

As implementações de AF-01 a AF-07 foram corrigidas neste worktree. O relatório original abaixo descreve o commit auditado, anterior às correções.

- Atualizador verifica assinatura Minisign, versão/nome assinados, release oficial e HTTPS; não procura nem transmite tokens GitHub. Sem chave pública/signature, usa a página oficial e recusa instalação no backend.
- Captura de download exige nonce, porta/caminho corretos, hosts oficiais e HTTPS, incluindo redirecionamentos. Nomes são validados e downloads/Vault usam diretórios temporários exclusivos.
- Comandos próprios têm ACL explícita da janela principal local. Projetos/thumbnails são validados e limitados às pastas monitoradas; thumbnails têm limite de 8 MB; exclusão usa a Lixeira; criação Vault rejeita nomes perigosos, e gravações protegidas recusam links/junctions.
- Credenciais Epic usam o cofre do sistema com migração e remoção do JSON após gravação confirmada. Falhas não criam fallback em texto puro. Migração/logout também passaram em um teste real do Credential Manager com credencial fictícia exclusiva, apagada ao final.
- `rustls` atualizado para `0.23.45`; versão do catálogo escapada antes de inserir HTML.
- Após correção, `npm audit`: zero vulnerabilidades; `cargo audit`: zero na categoria principal de vulnerabilidades, mantendo seis avisos de manutenção e um de unsoundness em `glib` da cadeia Linux, ausente no Windows.
- Testes: 25 Rust, 6 frontend e 2 de publicação assinada passaram; teste nativo de Credential Manager adicional passou. Integração com servidor Epic e publicação de release assinada não foram executadas com a conta real.

O build Windows de release e o instalador NSIS foram gerados com sucesso após os testes finais.

A configuração de chaves/secrets e os limites restantes estão em [SECURITY_UPDATES.md](SECURITY_UPDATES.md). Authenticode e a revisão da dependência `glib` para Linux continuam pendentes; não fazem parte do bloqueio de pacotes no atualizador.

## Resultado original da auditoria

Foram encontradas falhas de segurança no código e uma vulnerabilidade conhecida em dependência usada no Windows. Recomenda-se corrigir os downloads e o atualizador antes de distribuir novas versões. Esta revisão não é uma certificação de ausência de vulnerabilidades.

A auditoria não alterou a implementação, não publicou commits e não acessou o conteúdo das sessões reais Epic/GitHub. As severidades do código abaixo são avaliações contextuais, sem atribuição de CVSS. Algumas dependem de comprometimento prévio do frontend ou de acesso local; isso está explicitado em cada item.

## Verificações executadas

- Revisão dos comandos Tauri, execução de processos, login Epic, downloads, instalação do Vault, operações em arquivos, renderização HTML, CSP e workflow de release.
- `npm audit`: zero vulnerabilidades conhecidas nas dependências presentes no lockfile.
- `cargo audit 0.22.2`: 566 dependências no lockfile; base RustSec com 1.288 avisos, commit `f8dee89e1b2f2f1eaf548312df7655fe5202a302`.
- `cargo tree` para Windows: confirmou `rustls 0.23.43` no caminho de `reqwest` e `egs-api`; `glib` não faz parte desse alvo.
- Teste Rust isolado da função original `is_valid_engine_download_url`, extraída do código sem alterações, e das expressões de composição de caminhos. A escrita demonstrativa ficou inteiramente em `src-tauri/target/security-audit/fixtures`.
- `Get-AuthenticodeSignature`: instalador Windows local com estado `NotSigned`.
- Testes existentes: 17 testes Rust e 3 testes frontend passaram; 2 testes Rust de integração ficaram ignorados. Esses testes funcionais não cobrem todas as falhas de segurança descritas abaixo.

Resultados brutos e ferramenta de reprodução estão em `src-tauri/target/security-audit/`, ignorado pelo Git. A prova não executou instaladores, não baixou pacotes maliciosos e não excluiu projetos.

## Achados prioritários

### AF-01 — Alta: atualização executada sem autenticar o pacote

**Locais:** `src-tauri/src/updater.rs:208`, `:325`, `:334`; `src-tauri/src/commands.rs:652`.

`download_and_apply_update` recebe URL e nome pelo IPC, baixa os bytes e executa o `.exe` ou instala o `.msi`. Não verifica assinatura criptográfica, identidade do publicador, vínculo com a release consultada nem restringe HTTPS/origem. O fluxo normal pede um clique do usuário; não foi observada instalação silenciosa na inicialização.

**Condições e impacto:** comprometimento da release/distribuição ou controle do frontend autorizado ao IPC pode substituir a atualização por um executável controlado pelo atacante. Ele executa com os direitos do usuário; elevação depende do instalador/UAC. Não foi demonstrado um ataque remoto sem essas condições.

**Correção:** usar atualização assinada com chave pública incorporada, preferencialmente o plugin oficial Tauri, que exige assinatura; resolver o pacote no backend, aceitar apenas origem HTTPS esperada e validar redirecionamentos. Criar diretório temporário exclusivo, impedir separadores no nome e verificar o pacote antes de qualquer execução. O prefixo fixo em `temp_dir.join(format!(...asset_name))` também preserva `..` quando o nome contém separadores.

[Documentação de assinatura do updater Tauri](https://v2.tauri.app/plugin/updater/#signing-updates).

### AF-02 — Alta, condicional: token GitHub enviado à URL de download

**Locais:** `src-tauri/src/updater.rs:115`, `:219`.

O app procura `GITHUB_TOKEN`, `UNREAL_LAUNCHER_GITHUB_TOKEN` ou `oauth_token` na configuração do `gh`. Em seguida adiciona `Authorization: Bearer ...` à requisição inicial de download, qualquer que seja o host informado. Esse problema existe na requisição inicial, independentemente da política do cliente para remover cabeçalhos em redirecionamentos.

**Condições e impacto:** é necessário haver um token disponível e controle da URL por quem comprometeu o frontend ou os metadados de atualização. Um servidor externo recebe o token; o acesso à conta/repositórios depende dos escopos desse token. Não foi consultado nem enviado um token real nesta revisão.

**Correção:** downloads de release pública não precisam do token pessoal. Remover a descoberta automática de credenciais `gh`; se releases privadas forem necessárias, usar autorização explícita e limitada ao endpoint GitHub permitido, sem repassar credenciais a servidores de assets.

### AF-03 — Alta: capturador de download sem autenticação e gravação fora do destino

**Locais:** `src-tauri/src/epic.rs:531`, `:552`, `:612`, `:621`, `:804`; `src-tauri/src/engine.rs:496`, `:497`, `:533`, `:618`.

O listener de download aceita a primeira requisição com parâmetro `url` sem nonce/autenticação. A função de validação testa partes da string, permitindo qualquer host com `.zip`, HTTP sem TLS e endereços privados. O nome extraído de `response-content-disposition` segue para `dest.join(blob.name)` sem exigir um único nome de arquivo.

**Prova local:** a função original aceitou `https://attacker.invalid/engine.zip`, `http://attacker.invalid/engine.zip`, `https://attacker.invalid/ucs-blob-store` e `http://192.168.1.10/engine.zip`. A expressão usada pela Engine com nome `..\outside.zip` gravou o arquivo demonstrativo fora da subpasta escolhida, ainda dentro da pasta de testes.

**Condições e impacto:** um processo local que encontre a porta durante a captura pode trocar a URL. Controle do IPC ou de uma URL capturada também alcança a mesma validação. O usuário ainda precisa prosseguir com o download; o fluxo efetivamente usado no Windows pode encaminhar à Epic em vez de usar esse downloader. Quando o downloader interno é usado, o nome pode escapar do destino, sobrescrever/criar arquivos e o pacote pode ser de origem não oficial. A extração/registro não prova a autenticidade da Engine; a execução do Editor ocorre posteriormente.

**Correção:** aplicar nonce imprevisível no listener e no callback de navegação, validar URL por parser, exigir HTTPS e lista explícita de hosts oficiais, verificar redirecionamentos e rejeitar IPs privados. Gerar nome temporário no backend, rejeitar nomes absolutos, separadores, `..`, dispositivos Windows e ADS. Verificar destino e autenticidade antes de extrair. A extração de ZIP usa utilitários do sistema; não foi provado um bypass específico do `tar`/`Expand-Archive` nesta revisão.

### AF-04 — Média: comandos de arquivos ultrapassam o escopo de projetos

**Locais:** `src-tauri/src/commands.rs:248`, `:348`; `src-tauri/src/vault.rs:627`; `src-tauri/build.rs:1`.

`read_project_thumbnail` lê qualquer caminho recebido e retorna os bytes em base64, sem checar tipo, tamanho ou vínculo com projeto. `delete_project_from_disk` não exige extensão `.uproject`, descritor válido nem registro/escopo monitorado: qualquer arquivo existente em uma pasta não protegida pode servir de argumento para excluir a pasta inteira. A proteção atual bloqueia algumas pastas padrão exatas e a raiz. O caminho de criação pelo Vault aceita `project_name` com separadores, ao contrário da criação normal de projetos.

**Condições e impacto:** frontend comprometido ou outra chamada IPC autorizada pode ler arquivos acessíveis ao usuário, apagar pastas ou instalar conteúdo fora do destino esperado. Não se trata de acesso HTTP público; não há capabilities remotas configuradas. Não foi demonstrado um exploit de execução de JavaScript que alcance esses comandos.

**Correção:** identificar projetos no backend, canonicalizar e conferir o escopo selecionado pelo usuário, validar descritores e nomes, limitar thumbnail a formatos/tamanhos previstos e usar confirmação confiável/reversível para exclusão. Declarar permissões dos comandos próprios com `AppManifest::commands` e concedê-las somente à janela principal; as permissões atuais de plugins não substituem validação nos comandos Rust.

[Permissões dos comandos próprios no Tauri](https://v2.tauri.app/security/capabilities/).

### AF-05 — Média: sessão Epic armazenada em texto puro

**Locais:** `src-tauri/src/epic.rs:21`, `:85`, `:88`.

O JSON de sessão contém `access_token` e `refresh_token` sem criptografia. No Windows o arquivo usa as permissões herdadas; o bloco `0600` só existe para Unix. Um processo com acesso ao perfil ou uma cópia de backup desse arquivo pode obter as credenciais. Não foi testado acesso por outro usuário Windows; não há evidência de que todos os usuários tenham acesso ao arquivo.

**Correção:** armazenar tokens com Windows Credential Manager/DPAPI e equivalente seguro no Linux; migrar e remover o JSON antigo, evitar tokens em logs e tratar falha de persistência/logout. Criptografia vinculada ao usuário protege o arquivo em repouso, mas não neutraliza um processo malicioso já executando na mesma conta.

### AF-06 — Média: dependência TLS com vulnerabilidade conhecida

**Local:** `src-tauri/Cargo.lock`, pacote `rustls 0.23.43`.

`cargo audit` identificou **RUSTSEC-2026-0285 / GHSA-2mjx-qc3c-rqvc**, CVSS 5.3. A biblioteca aceita determinadas mensagens TLS 1.3 no nível de criptografia errado. O transcript permanece autenticado: o advisory não afirma que um atacante de rede consegue alterar ou completar o handshake. A dependência faz parte do aplicativo Windows via dois caminhos de `reqwest`.

**Correção:** atualizar o lockfile para `rustls >= 0.23.45`, reconstruir, testar login/downloads e repetir o audit. Atualizar somente o lockfile não corrige instaladores já distribuídos.

[RustSec oficial](https://rustsec.org/advisories/RUSTSEC-2026-0285.html) e [advisory do mantenedor](https://github.com/rustls/rustls/security/advisories/GHSA-2mjx-qc3c-rqvc).

### AF-07 — Baixa: versão de catálogo interpolada como HTML

**Locais:** `src/main.js:1091`, `:1113`.

`blob.version` (ou o nome do blob) forma `cleanName`, inserido diretamente em `innerHTML`. A maior parte da UI já usa `escapeHtml`, mas essa lista não. Metadados de catálogo/cache adulterados podem inserir marcação e alterar a apresentação. A CSP atual não permite handlers inline; esta revisão não demonstrou execução de JavaScript, portanto o achado é injeção HTML, não uma cadeia XSS→IPC confirmada.

**Correção:** usar `textContent`/DOM ou `escapeHtml` e validar formato de versão no backend. Escapar também strings interpoladas em scripts de captura, como `target_version` em `epic.rs:675`.

## Alertas adicionais e limites

- O audit sinalizou `glib 0.18.5` com **RUSTSEC-2024-0429** (aviso de unsoundness). Não aparece na árvore Windows. Para Linux, atualizar a cadeia GTK/Tauri compatível e verificar se `VariantStrIter` é usado; não foi comprovada alcançabilidade no aplicativo. [Aviso oficial](https://rustsec.org/advisories/RUSTSEC-2024-0429.html).
- Seis pacotes tiveram avisos de manutenção: `proc-macro-error`, `unic-char-property`, `unic-char-range`, `unic-common`, `unic-ucd-ident`, `unic-ucd-version`. Esses avisos não equivalem a seis vulnerabilidades exploráveis. Planejar atualização dos dependentes.
- O Vault rejeita caminhos relativos com `..`, raiz e drive, mas ainda merece testes com junctions/symlinks preexistentes e diretórios temporários previsíveis. Não foi provada exploração por links nesta rodada.
- As URLs pré-assinadas de download são impressas no terminal (`engine.rs:506` e capturador Epic). São credenciais temporárias de acesso ao objeto; removê-las ou mascarar query strings antes de compartilhar logs.
- O instalador local não tem Authenticode. Isso é uma lacuna de autenticidade/distribuição, não evidência de malware ou garantia de bloqueio de conta. Assinatura de updater e assinatura Windows têm funções diferentes.
- O workflow usa referências mutáveis de Actions (`@v4`, `@v2`, `@stable`, `@v0`). Fixar SHA e adicionar auditorias de dependências à CI melhora proteção da cadeia de distribuição; não há evidência de workflow comprometido.
- O build Windows já restringe metacaracteres de `cmd.exe` e valida nomes de targets. Compilar/abrir um projeto Unreal executa código do projeto (por exemplo regras `.Build.cs`); essa função pressupõe projetos confiáveis, como a IDE/UnrealBuildTool.
- A CSP bloqueia scripts remotos/inline e não há permissão remota configurada. Não foi encontrada uma cadeia remota completa de execução de código; as falhas de validação confirmadas não devem ser descritas como comprometimento já ocorrido.
- Não foram auditados binários da Unreal, WebView2 instalado no computador, histórico completo de segredos Git, servidores Epic/GitHub, nem realizados pentest externo ou testes destrutivos. Nenhuma credencial real foi usada em provas de exploração.

## Ordem recomendada de correção

1. AF-01/AF-02: atualização autenticada e remoção do repasse de token pessoal.
2. AF-03/AF-04: autenticar captura, validar URLs/nomes/caminhos e limitar comandos de arquivos.
3. AF-06: atualização pontual de `rustls`, rebuild e novo audit.
4. AF-05/AF-07: proteção dos tokens e renderização segura de metadados.
5. Testes de regressão para essas barreiras, auditorias na CI e assinatura dos artefatos de distribuição.
