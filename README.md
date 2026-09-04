# Unreal Launcher (Linux)

Launcher não-oficial pra gerenciar instalações da Unreal Engine e projetos no Linux,
já que a Epic não distribui um launcher nativo pra essa plataforma.

## O que ele faz (v1)

- **Engines**: registra instalações da UE já extraídas, ou extrai um `.zip` baixado em
  [unrealengine.com/linux](https://www.unrealengine.com/linux?lang=pt-BR) (build "Installed Build",
  pré-compilada — sem precisar compilar o source, que é gigante). Detecta a versão lendo
  `Engine/Build/Build.version`.
- **Projetos**: monitora pastas, escaneia recursivamente por `.uproject`, lê o campo
  `EngineAssociation` de cada projeto e tenta casar automaticamente com uma engine registrada
  (por versão). Se não achar, você escolhe manualmente no dropdown. Botão "Abrir" lança
  `Engine/Binaries/Linux/UnrealEditor <projeto>.uproject` de forma desacoplada do launcher.
- **Criar projeto novo**: cada engine tem um botão "Abrir Editor" que abre o editor sem nenhum
  projeto — cai direto no Project Browser nativo da Unreal, de onde dá pra criar um projeto novo
  ou abrir um existente manualmente.
- **Abrir código C++ na IDE**: projetos com pasta `Source/` ganham um badge "C++" e um botão
  "Código" com seletor de IDE (VS Code, Rider, CLion). Pro VS Code, se existir um arquivo
  `.code-workspace` na raiz do projeto (gerado pelo UBT via "Generate Project Files"), ele abre
  esse workspace em vez da pasta crua — assim os include paths e build tasks já vêm configurados.
- **Thumbnail e remoção de projetos**: cada card de projeto mostra a captura de tela automática
  que a própria Unreal salva em `Saved/AutoScreenshot.png` (a mesma usada no Project Browser
  nativo); sem ela, mostra um placeholder com as iniciais do projeto. O botão "×" remove o
  projeto da lista — ele entra numa lista de exclusão no config, os arquivos no disco não são
  tocados; se quiser fazer aparecer de novo, é só editar `excluded_projects` no
  `~/.config/unreal-launcher/config.json`.
- **Login com a conta Epic (Nativo em Rust)**: a sidebar mostra o status da conta e um botão
  "Entrar com Epic". A autenticação é realizada **100% nativamente em Rust via HTTP**, sem precisar
  instalar ferramentas externas (como Python, pipx ou Legendary).
  O login é **automático**: o launcher abre uma janela embutida (webview do próprio Tauri) com a
  página oficial de login da Epic; assim que ela navega para a página final com o código, o launcher
  captura sozinho, troca o `authorizationCode` por tokens OAuth e salva a sessão em
  `~/.config/unreal-launcher/epic_session.json` (com renovação automática via `refresh_token`).
  Se a captura automática falhar, há um campo "Colar o código manualmente" como plano B.
- **Download Direto da Unreal Engine (Linux)**: conectado à conta Epic, o launcher consulta a
  Cosmos API oficial (`unrealengine.com/api/blobs/linux`) e lista todas as compilações pré-compiladas
  oficiais disponíveis (Installed Builds). O launcher faz o download do `.zip` diretamente da AWS S3
  com indicador de velocidade (MB/s) e barra percentual, executa a extração automaticamente no
  diretório escolhido e já registra a nova engine no `Install.ini`.
- **Integração com Rider / VS Code / UnrealBuildTool**: toda vez que uma engine é registrada ou
  removida, o launcher também escreve `~/.config/Epic/UnrealEngine/Install.ini` — é o arquivo que
  o Epic Games Launcher normalmente mantém, e que ferramentas como o plugin Unreal do Rider, o
  UnrealVS e o próprio `UnrealBuildTool` leem pra resolver o campo `EngineAssociation` de um
  `.uproject` (ex: `"5.8"`) pro caminho real da engine no disco. Sem isso, mesmo com a associação
  certa no `.uproject`, essas ferramentas simplesmente não acham a engine — só o nosso launcher
  sabia da associação, guardada no config interno dele.

## Download e Instalação da Engine

Você pode adicionar engines de 3 formas:
1. **Baixar direto da Epic Games**: clique em "Baixar da Epic Games ⤓", escolha a versão na lista oficial e aponte a pasta de instalação. O launcher faz o download, descompacta e registra tudo sozinho.
2. **Já está extraída**: selecione a pasta da engine já descompactada no disco.
3. **Extrair um `.zip` local**: selecione um arquivo `.zip` baixado anteriormente e a pasta de destino.

## Rodando em desenvolvimento

Pré-requisitos (Ubuntu/Debian):

```bash
# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Dependências do Tauri (webview + build)
sudo apt update
sudo apt install -y libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev unzip

# CLI do Tauri
cargo install tauri-cli --version "^2"
```

Pré-requisitos (Rocky Linux / RHEL / Fedora — via `dnf`):

```bash
# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Rocky/RHEL usam repositórios mais enxutos que o Fedora — habilite o EPEL primeiro
sudo dnf install -y epel-release
sudo dnf config-manager --set-enabled crb   # "CodeReady Builder" no Rocky 10, necessário pro webkit2gtk-devel

sudo dnf install -y webkit2gtk4.1-devel openssl-devel curl wget file \
  libappindicator-gtk3-devel librsvg2-devel libxdo-devel unzip \
  gcc gcc-c++ make rpm-build

# CLI do Tauri
cargo install tauri-cli --version "^2"
```

Rodar em modo dev (hot reload do frontend, já que é HTML/CSS/JS puro):

```bash
cargo tauri dev
```

## Automatizando pelo VS Code

Tem um `.vscode/tasks.json` já configurado com tasks (acessíveis por `Ctrl+Shift+P` →
`Tasks: Run Task`):

- **Setup: Ambiente completo** — roda as duas de baixo em sequência, pra deixar uma máquina nova
  pronta pra compilar (rodar uma vez só, na primeira vez que abrir o projeto numa distro nova):
  - **Setup: Instalar dependências do sistema (dnf)** — instala de uma vez toda a stack GTK/WebKit,
    dbus, fuse e as ferramentas de build/empacotamento que fomos descobrindo ao longo do
    troubleshooting (webkit2gtk, glib2, gtk3, cairo, pango, atk, dbus, libxdo, fuse, gcc, rpm-build
    etc). Em distros Fedora-based (Nobara, Fedora) não precisa de EPEL/CRB antes — isso só é
    necessário em RHEL/Rocky.
  - **Setup: Instalar tauri-cli** — `cargo install tauri-cli --version "^2"`.
- **Tauri: Dev** — roda `cargo tauri dev` (modo desenvolvimento, hot reload do frontend).
- **Tauri: Build (RPM)** — roda `cargo tauri build`, só compila e empacota.
- **Tauri: Build + Instalar (RPM)** — build seguido de `sudo dnf reinstall`/`install` do `.rpm`
  gerado. É a task **default de build**, então `Ctrl+Shift+B` já dispara ela direto.

Como usa `sudo`, o terminal integrado do VS Code vai pedir sua senha ali mesmo na hora —
é normal, digita e segue.

> Nota: as tasks assumem o target `rpm` (ajustado em `tauri.conf.json` por causa de um problema
> com `linuxdeploy`/FUSE no AppImage nesse ambiente — ver seção anterior). Se resolver o AppImage
> depois, é só adicionar `"appimage"` de volta em `bundle.targets` e ajustar o glob do `.rpm` na
> task de instalar, se quiser automatizar aquele também.

## Gerando o instalador manualmente (sem VS Code)

```bash
cargo tauri build
```

Isso compila em modo release e empacota os dois formatos configurados em
`tauri.conf.json` (`bundle.targets`). Os artefatos ficam em:

```
src-tauri/target/release/bundle/appimage/unreal-launcher_0.1.0_amd64.AppImage
src-tauri/target/release/bundle/rpm/unreal-launcher-0.1.0-1.x86_64.rpm
```

- **AppImage**: arquivo único, portátil. Dá `chmod +x` e roda direto — não precisa instalar nada.
  Se der erro de FUSE ao rodar (`dlopen(): error loading libfuse.so.2`), instale `fuse-libs`
  (`sudo dnf install fuse-libs`) ou rode com `--appimage-extract-and-run`.
- **RPM**: instala de verdade no sistema, integra com o menu do KDE (ícone, `.desktop` file):
  ```bash
  sudo dnf install ./src-tauri/target/release/bundle/rpm/unreal-launcher-0.1.0-1.x86_64.rpm
  ```
  Depois disso ele aparece no launcher de aplicativos do Plasma normalmente, sem precisar do
  terminal nem do VS Code pra abrir.

> O ícone em `src-tauri/icons/icon.png` é só um placeholder gerado programaticamente — troque
> por uma arte de verdade antes de distribuir (o `.desktop` gerado no RPM usa esse ícone).

## Estrutura

```
unreal-launcher/
├── src/                      # frontend (HTML/CSS/JS puro, sem bundler)
│   ├── index.html
│   ├── styles.css
│   └── main.js
└── src-tauri/                # backend Rust
    ├── src/
    │   ├── main.rs            # bootstrap do Tauri, registro de comandos
    │   ├── config.rs          # persistência em ~/.config/unreal-launcher/config.json
    │   ├── engine.rs          # detecção de engine + extração de zip com progresso
    │   ├── project.rs         # scan de .uproject + matching + launch
    │   └── commands.rs        # comandos #[tauri::command] expostos ao frontend
    ├── capabilities/           # permissões dos plugins (dialog, opener, shell)
    └── tauri.conf.json
```

## Onde a config fica salva

`~/.config/unreal-launcher/config.json` — lista de engines registradas e pastas de projetos
monitoradas. É só JSON, dá pra editar na mão se precisar.

## Roadmap (não implementado ainda, pra quando quiser expandir)

- **Criar projeto novo a partir de template**, sem precisar abrir o editor primeiro.
- **Build/package via linha de comando** (`RunUAT.sh BuildCookRun`) direto pela UI, com log em
  tempo real (mesmo padrão de streaming de progresso já usado na extração do zip).
- **Gerenciar plugins por projeto** — listar `.uplugin` em `Plugins/`, habilitar/desabilitar.
- **Build da engine a partir do source** — clone + `Setup.sh` + `GenerateProjectFiles.sh` + `make`,
  com log streaming; fica pesado (build de horas) então merece uma fila/worker separado do processo
  principal em vez de rodar inline num comando Tauri.
- **Multi-conta / múltiplos discos**: hoje os caminhos são absolutos; se você reorganizar pastas,
  a engine ou projeto some da lista até re-registrar.
