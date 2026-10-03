# ArcForge

Launcher independente para Unreal Engine no **Windows e Linux**, feito com Tauri 2, Rust e JavaScript. Versão atual: **0.1.5**.

[Baixar a release](https://github.com/ricardofuly/ArcForge/releases/latest) · [Segurança e assinaturas](SECURITY_UPDATES.md) · [Revisão de segurança](SECURITY_REVIEW.md)

## O que o launcher faz

- Interface ArcForge com navegação por abas, fundo personalizado e controles de minimizar, maximizar e fechar integrados.
- Dashboard com projetos recentes e conta Epic na parte inferior.
- Aba **Engines** com instalações registradas, abertura do Editor e cartão de download sempre ao final da lista. No Windows, o download usa o fluxo oficial da Epic Games Launcher; no Linux, há suporte aos ZIPs oficiais de builds pré-compilados.
- Aba **Configurações** para registrar Engines e pastas de projetos monitoradas.
- Projetos Blueprint e C++, criação de projetos, escolha da Engine e seleção de RHI Auto/SM5/SM6. Integração com VS Code, Rider, CLion e Visual Studio conforme a disponibilidade no sistema.
- Biblioteca Epic/Vault com download e instalação de conteúdo em projetos e Engines e criação de projetos a partir de conteúdo compatível.
- Downloads com progresso e verificação de novas releases na aba Configurações, com diálogo de atualização integrado ao visual do ArcForge.

O ArcForge não é um produto oficial da Epic Games. Downloads e conteúdo da conta continuam sujeitos à disponibilidade e às permissões da Epic/Fab.

## Compilação C++ antes de abrir

Ao clicar em **Iniciar Projeto** em um projeto C++, o launcher executa o build do alvo Editor pelo UnrealBuildTool antes de iniciar a Unreal. A compilação é incremental: arquivos já atualizados podem não precisar ser recompilados, mas a verificação de build acontece em cada abertura.

A interface mostra o andamento e o log. Se o build falhar, o Editor não é iniciado e os erros ficam disponíveis. O log também é salvo em `Saved/Logs/LauncherBuild-<id>.log` dentro do projeto. Os artistas não precisam abrir uma IDE para compilar.

No Windows, a máquina ainda precisa da Engine e do compilador MSVC/Windows SDK compatíveis com sua versão da Unreal, instalados pelo Visual Studio ou Build Tools com as ferramentas C++. No Linux, são necessárias as ferramentas de compilação exigidas pela Engine. O launcher não instala esses compiladores automaticamente.

## Instalação

Baixe o instalador Windows `.exe` ou `.msi` na release. Para Linux, use os pacotes `.deb` ou `.AppImage` disponibilizados pela release. O Windows usa WebView2; o Linux precisa das bibliotecas GTK/WebKit e de um serviço Secret Service ativo, como GNOME Keyring ou KWallet, para guardar a sessão Epic.

Abra **Configurações**, registre sua pasta de Engine caso ela não seja detectada e adicione as pastas dos seus projetos. Depois use **Projetos** para escolher a Engine e iniciar o trabalho.

## Executar pelo terminal

Pré-requisitos: Git, Node.js/npm, Rust/Cargo e as dependências de desenvolvimento do Tauri para o sistema. No Windows, use a toolchain Rust MSVC e instale as ferramentas C++/Windows SDK.

```powershell
git clone https://github.com/ricardofuly/ArcForge.git
cd ArcForge
npm ci
npm run tauri dev
```

Se aparecer `cargo metadata: program not found`, instale o Rust, feche e reabra o terminal e confira `cargo --version`. Em instalações padrão, o Cargo está em `%USERPROFILE%\.cargo\bin`, que precisa estar no `PATH`.

No Ubuntu, as dependências usadas pelo workflow são:

```bash
sudo apt-get install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libssl-dev
npm ci
npm run tauri dev
```

## Gerar instaladores

Incorpore a chave pública antes do build para habilitar a verificação das atualizações:

```powershell
$env:ARCFORGE_UPDATE_PUBLIC_KEY = (Get-Content arcforge.pub)[1].Trim()
npm run tauri build -- --bundles nsis,msi
```

Os pacotes Windows ficam em `src-tauri/target/release/bundle/nsis/` e `src-tauri/target/release/bundle/msi/`.

No Linux:

```bash
export ARCFORGE_UPDATE_PUBLIC_KEY="$(sed -n '2p' arcforge.pub)"
npm run tauri build -- --bundles deb,appimage
```

Os pacotes ficam em `src-tauri/target/release/bundle/`. O workflow de release constrói Windows e Linux, testa, audita e mantém a release em rascunho até verificar as assinaturas.

## Live Update

No Windows, o ArcForge baixa o executável assinado, verifica sua assinatura, aguarda o processo atual encerrar, substitui o aplicativo e reinicia no mesmo caminho. A interface mostra download, verificação e preparação. Falhas na substituição ou na criação do novo processo restauram a versão anterior; o atualizador mostra o erro e o caminho do log. Projetos, configurações e sessão Epic permanecem nos respectivos diretórios de dados.

As novas instalações NSIS são feitas para o usuário atual. Instalações em pastas protegidas, como Program Files (MSI), precisam ser migradas para uma instalação por usuário para usar este fluxo sem administrador. O app verifica a permissão antes de fechar. No Linux, Live Update está disponível para AppImage; DEB/RPM continuam pelo gerenciador de pacotes.

O workflow publica `ArcForge_<versão>_windows_x64.bin` e sua assinatura `.minisig`, além dos instaladores. O `.bin` contém o executável completo com a interface incorporada e é exclusivo do atualizador. As versões até 0.1.4 precisam instalar uma versão com este novo mecanismo uma vez; nas atualizações seguintes, não é necessário abrir o instalador. Sem um pacote compatível assinado, o app oferece a página oficial da release.

## Segurança e verificação dos pacotes

O atualizador aceita apenas pacotes da última release oficial, via HTTPS, com assinatura **Minisign** válida e comentário assinado correspondente à versão e ao nome do arquivo. A chave pública está em [arcforge.pub](arcforge.pub). Cada instalador publicado acompanha um arquivo `.minisig`.

Com ambos os arquivos na mesma pasta e a chave pública obtida deste repositório:

```powershell
minisign -V -p arcforge.pub -m ArcForge_0.1.5_x64-setup.exe
```

Minisign autentica o pacote para o atualizador. Não substitui Authenticode: os instaladores ainda não possuem certificado de publicador Windows. Aplicativos antigos precisam receber este instalador para passar a usar a nova verificação.

A sessão Epic é guardada no Windows Credential Manager ou Secret Service, sem escrita de novos tokens em JSON. Sessões antigas são migradas antes da remoção do arquivo legado. O atualizador público não lê tokens pessoais do GitHub.

Na versão 0.1.4, a biblioteca exige login antes de ler o cache e vincula os itens à conta Epic. Sair da conta limpa os itens e seu cache; respostas pendentes de uma sessão anterior são descartadas. Caches antigos sem identificação da conta são ignorados e recriados ao sincronizar. Esses arquivos são locais e não acompanham os instaladores.

As operações protegidas validam nomes e caminhos, recusam links/junctions e limitam downloads e thumbnails. A exclusão de projetos usa a Lixeira. Projetos e plugins Unreal precisam ser confiáveis: o compilador e o Editor executam código deles.

A revisão corrigiu os achados documentados e atualizou `rustls`. Permanece uma advertência de segurança em `glib` na árvore Linux, cuja exposição ainda precisa ser investigada, além de avisos de dependências sem manutenção. Não há garantia de ausência de vulnerabilidades. Consulte [SECURITY_REVIEW.md](SECURITY_REVIEW.md) para os limites da análise.

## Testes

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --locked
node --test tests/frontend-build.test.cjs tests/security-ui.test.cjs tests/vault-auth.test.cjs
python -m unittest discover -s tests -p test_security_release.py
npm audit
cargo audit --file src-tauri/Cargo.lock
```

A validação local passou com 25 testes Rust, 9 testes de interface e 3 testes de publicação; três integrações ficam ignoradas na execução padrão. O teste isolado do Credential Manager também foi executado na revisão de segurança. A autenticação/download real da Epic não foi retestada nessa revisão.

## Dados e compatibilidade

A configuração continua em `%APPDATA%\unreal-launcher\config.json` no Windows e `~/.config/unreal-launcher/config.json` no Linux. O identificador interno legado foi mantido para preservar a compatibilidade; o nome exibido é ArcForge.

O código-fonte está em `src/` (interface), `src-tauri/src/` (backend) e `tests/`. As instruções de chaves, publicação e migração de credenciais estão em [SECURITY_UPDATES.md](SECURITY_UPDATES.md).

