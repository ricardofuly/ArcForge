# Unreal Launcher (Linux) — Documento de Contexto Técnico & Arquitetura

Este documento serve como referência central para entender o propósito, a estrutura técnica atual do **Unreal Launcher**, o diagnóstico da integração com a Epic Games e o plano detalhado para permitir o login direto e download automatizado da Unreal Engine nativa para Linux.

---

## 1. Visão Geral do Projeto

### 1.1 O Problema
A Epic Games **não disponibiliza** uma versão nativa do *Epic Games Launcher* para distribuições Linux. Desenvolvedores que utilizam Linux tradicionalmente enfrentam fricções significativas:
1. Precisam compilar a engine a partir do código-fonte do GitHub (processo que pode levar horas e consumir mais de 100GB de disco).
2. Ou precisam acessar manualmente o portal web da Epic Games (`unrealengine.com/linux`), fazer login no navegador, baixar manualmente um arquivo compactado (`.zip`) de 30 a 40 GB de uma "Installed Build" pré-compilada, descompactá-lo e configurar os executáveis manualmente.
3. Não há uma ferramenta nativa de sistema para associar automaticamente arquivos de projeto (`.uproject`) com as engines instaladas, gerenciar arquivos de workspace de IDEs (VS Code, JetBrains Rider, CLion) ou atualizar o arquivo de registro do sistema (`~/.config/Epic/UnrealEngine/Install.ini`).

### 1.2 A Solução: Unreal Launcher
O **Unreal Launcher** é uma aplicação desktop nativa para Linux, rápida e moderna, desenvolvida com **Tauri 2 (Rust + Web Frontend)**. O launcher tem como objetivo:
- Registrar, gerenciar e executar diferentes versões pré-compiladas (Installed Builds) ou compiladas do fonte (Source Builds) da Unreal Engine.
- Escanear pastas e organizar projetos Unreal (`.uproject`), identificando a engine associada e permitindo alterar a versão de execução.
- Lançar projetos ou o editor vazio de forma desacoplada do processo do launcher.
- Integrar com IDEs populares (VS Code, Rider, CLion), abrindo diretamente `.code-workspace` quando aplicável para garantir include paths e tarefas de compilação prontas.
- Manter o ecossistema de desenvolvimento do sistema consistente, gerando automaticamente o `Install.ini` para que plugins de IDEs e ferramentas de build como a `UnrealBuildTool` (UBT) localizem as engines registradas.

---

## 2. Arquitetura Atual do Código

A aplicação adota a arquitetura de processo do **Tauri 2**:
- **Backend (Rust)**: Responsável por I/O no sistema de arquivos, execução de processos desacoplados, persistência de configuração, comunicação com o sistema operacional e rotinas bloqueantes/multithread.
- **Frontend (Web)**: Interface de usuário limpa, escura e responsiva, desenvolvida em HTML5, CSS3 moderno e JavaScript Vanilla (sem a sobrecarga de bundlers pesados como Vite ou Webpack).

### 2.1 Estrutura de Diretórios
```
unreal-launcher/
├── src/                                  # Frontend Web
│   ├── index.html                        # Estrutura de views, modais e layouts
│   ├── styles.css                        # Design system escuro, tipografia e componentes
│   └── main.js                           # Lógica de interface, eventos Tauri e IPC
├── src-tauri/                            # Backend Rust (Tauri 2)
│   ├── Cargo.toml                        # Dependências Rust
│   ├── tauri.conf.json                   # Configurações do Tauri, janelas e bundles (RPM/AppImage)
│   ├── capabilities/                     # Permissões do Tauri (shell, dialog, opener)
│   └── src/
│       ├── main.rs                       # Ponto de entrada, plugins Tauri e registro de comandos
│       ├── config.rs                     # Estado global e persistência (~/.config/unreal-launcher/config.json)
│       ├── engine.rs                     # Detecção de engines, extração de ZIP e Install.ini
│       ├── project.rs                    # Varredura de .uproject, detecção C++, thumbnails e IDEs
│       ├── commands.rs                   # Handlers assíncronos #[tauri::command] expostos ao JS
│       └── epic.rs                       # Módulo atual de autenticação Epic (baseado em Legendary)
├── .vscode/tasks.json                    # Tarefas de ambiente, build RPM e execução
├── README.md                             # Documentação geral e guia de empacotamento
└── CONTEXTO.md                           # Este documento de referência
```

### 2.2 Módulos do Backend Rust

#### `main.rs`
- Inicializa a aplicação Tauri com os plugins oficiais (`tauri-plugin-shell`, `tauri-plugin-dialog`, `tauri-plugin-opener`).
- Injeta o estado compartilhado protegido por mutex (`AppState`).
- Registra todos os manipuladores de comando IPC definidos em `commands.rs`.

#### `config.rs`
- Gerencia o estado persistido em `~/.config/unreal-launcher/config.json`.
- Estrutura:
  - `engines`: Lista de engines registradas (`id`, `version`, `path`, `is_source_build`, `editor_bin`).
  - `project_dirs`: Lista de diretórios monitorados onde projetos `.uproject` residem.
  - `excluded_projects`: Lista de caminhos de projetos ocultados pelo usuário na UI.

#### `engine.rs`
- **Detecção de Engine**: Lê o arquivo `Engine/Build/Build.version` para obter `MajorVersion`, `MinorVersion`, `PatchVersion` e o hash `Changelist`. Localiza o executável do editor em `Engine/Binaries/Linux/UnrealEditor`.
- **Extração de ZIP**: Executa descompressão assíncrona com monitoramento de progresso através da ferramenta de sistema `unzip`, emitindo eventos periódicos `engine-extract-progress` com porcentagem e nome do arquivo atual para o frontend.
- **Sincronização com `Install.ini`**: Atualiza `~/.config/Epic/UnrealEngine/Install.ini` na seção `[Installations]`, garantindo que o `UnrealBuildTool` (UBT) e plugins de IDEs (como o Unreal Engine plugin do JetBrains Rider) consigam resolver a engine associada ao projeto.
- **Lançamento do Editor**: Executa `UnrealEditor` em um processo desacoplado (`Stdio::null`), permitindo fechar ou reiniciar o launcher sem interromper o editor.

#### `project.rs`
- **Varredura Recursiva**: Utiliza a biblioteca `walkdir` para localizar arquivos `.uproject` dentro dos diretórios monitorados.
- **Parsing de Projeto**: Lê o campo `EngineAssociation` do `.uproject` e tenta casar com uma engine registrada que possua a mesma versão (ou versão prefixada).
- **Projetos C++**: Detecta a existência do diretório `Source/`. Se existir, habilita o seletor de IDEs.
- **Integração com IDEs**:
  - *VS Code*: Procura por `.code-workspace` na raiz do projeto (gerado pelo UBT via "Generate Project Files"). Se presente, abre o workspace configurado; caso contrário, abre a pasta do projeto.
  - *Rider*: Lança `rider <caminho_do_projeto>`.
  - *CLion*: Lança `clion <caminho_do_projeto>`.
- **Thumbnails**: Lê assíncronamente o screenshot gerado pela Unreal Engine em `Saved/AutoScreenshot.png`, converte para Base64 e renderiza dinamicamente no card do projeto.

#### `epic.rs` (Estado Atual)
- Utiliza a ferramenta de linha de comando externa `legendary` (`legendary-gl`).
- Verifica status (`legendary status`) e autenticação (`legendary auth --code <código>`).
- Abre uma janela embutida do Tauri Webview apontando para a URL de login da Epic:
  `https://www.epicgames.com/id/login?redirectUrl=https%3A%2F%2Fwww.epicgames.com%2Fid%2Fapi%2Fredirect%3FclientId%3D34a02cf8f4414e29b15921876da36f9a%26responseType%3Dcode`
- Captura o código de autorização através de um script injetado na janela (`eval`) que escreve o código no título da janela e o Rust lê via `.title()`.

---

## 3. Diagnóstico da Integração Atual com a Epic Games

### 3.1 Limitações da Abordagem Atual via Legendary
1. **Dependência Externa Frágil**: O Legendary é um pacote Python que precisa ser instalado via `pipx` ou repositórios da distro. No Linux (especialmente em ambientes Fedora/RHEL/Nobara ou containers como Flatpak), o PATH do `pipx` frequentemente não está configurado, quebrando a integração com a mensagem "Legendary não encontrado".
2. **O Legendary NÃO baixa a Unreal Engine para Linux**:
   - O Legendary foi concebido como cliente da **Epic Games Store (EGS)**.
   - Os manifests da Epic Games Store distribuem pacotes da Unreal Engine exclusivamente para plataformas `Win64` e `Mac`.
   - Se o usuário tentar rodar comandos de instalação via Legendary para a Engine, ele fará o download da versão do Windows (`UnrealEditor.exe`), inutilizável nativamente no Linux para desenvolvimento de alta performance sem emulação pesada (Wine/Proton).
3. **Download Manual e Fora do Fluxo**:
   - O launcher atualmente apenas redireciona o usuário para abrir o navegador no site da Epic (`unrealengine.com/linux`), exigindo que o usuário faça download manual e depois selecione o arquivo baixado.

---

## 4. A Descoberta: Como Baixar a Unreal Engine Nativa Diretamente da Epic

Através de engenharia reversa das chamadas do portal oficial da Unreal Engine e análise de ferramentas da comunidade Linux (como o `Epic Asset Manager` e a biblioteca Rust `egs-api`), descobrimos exatamente como a Epic Games disponibiliza as compilações nativas de Linux.

### 4.1 A API Cosmos e os Blobs de Linux
A Epic Games disponibiliza as builds pré-compiladas oficiais de Linux através de uma API REST interna no domínio da Unreal Engine:
```http
GET https://www.unrealengine.com/api/blobs/linux
```

#### Resposta da API:
Quando autenticado com uma sessão ativa da Epic Games, esse endpoint retorna uma lista estruturada de todas as versões disponíveis da engine para Linux:
```json
{
  "blobs": [
    {
      "name": "Linux_Unreal_Engine_5.5.4.zip",
      "createdAt": "2025-02-15T14:30:00Z",
      "size": 34359738368,
      "url": "https://epicgames-engine-builds.s3.amazonaws.com/...presigned-s3-url..."
    },
    {
      "name": "Linux_Unreal_Engine_5.4.4.zip",
      "createdAt": "2024-09-10T10:00:00Z",
      "size": 32212254720,
      "url": "https://epicgames-engine-builds.s3.amazonaws.com/...presigned-s3-url..."
    }
  ]
}
```
> **Propriedade Chave:** O campo `url` contém uma URL pré-assinada da **AWS S3** temporária, que permite download direto em alta velocidade, sem passar por proxies ou exigir ferramentas de linha de comando adicionais!

### 4.2 O Fluxo Completo de Autenticação (Cosmos Session Handshake)

Para que a chamada ao endpoint `/api/blobs/linux` seja autorizada, é necessário realizar o seguinte aperto de mão de autenticação:

```mermaid
sequenceDiagram
    autonumber
    actor User as Usuário
    participant Webview as Tauri Webview
    participant Rust as Unreal Launcher (Rust)
    participant EpicOAuth as Epic Account Service
    participant EpicID as Epic ID (epicgames.com)
    participant Cosmos as Unreal Engine API (unrealengine.com)
    participant S3 as AWS S3 Storage

    User->>Webview: Realiza login na Epic
    Webview->>Rust: Captura authorizationCode
    Rust->>EpicOAuth: POST /account/api/oauth/token (authorization_code)
    EpicOAuth-->>Rust: Retorna access_token e refresh_token
    Rust->>EpicOAuth: GET /account/api/oauth/exchange (com access_token)
    EpicOAuth-->>Rust: Retorna exchange_code
    Rust->>EpicID: GET /id/api/reputation (captura cookie XSRF-TOKEN)
    Rust->>EpicID: POST /id/api/exchange (exchange_code + X-XSRF-TOKEN)
    Rust->>EpicID: GET /id/api/redirect (obtém sid - Session ID)
    Rust->>Cosmos: GET /id/api/set-sid?sid={sid} (registra cookies de sessão)
    Rust->>Cosmos: GET /api/cosmos/auth (valida e atualiza EPIC_EG1 tokens)
    Rust->>Cosmos: GET /api/cosmos/eula/accept?eulaId=unreal_engine2&locale=en
    Rust->>Cosmos: GET /api/blobs/linux
    Cosmos-->>Rust: Retorna lista de builds com URLs pré-assinadas da S3
    Rust->>S3: Download com streaming de progresso e barra percentual
    Rust->>Rust: Extrai ZIP via engine::extract_engine_zip
    Rust->>Rust: Registra engine e atualiza Install.ini
```

### 4.3 Detalhes Técnicos dos Endpoints

1. **Captura do Authorization Code**:
   - URL de login (já em uso no launcher):
     `https://www.epicgames.com/id/login?redirectUrl=https%3A%2F%2Fwww.epicgames.com%2Fid%2Fapi%2Fredirect%3FclientId%3D34a02cf8f4414e29b15921876da36f9a%26responseType%3Dcode`
   - Client ID: `34a02cf8f4414e29b15921876da36f9a` (o client ID público do launcher oficial da Epic).

2. **Obtenção do Token OAuth**:
   - Endpoint: `POST https://account-public-service-prod03.ol.epicgames.com/account/api/oauth/token`
   - Header: `Authorization: Basic MzRhMDJjZjhmNDQxNGUyOWIxNTkyMTg3NmRhMzZmOWE6ZGFhZmJjY2M3Mzc3NDUwMzkxZmZlNWQ5NGZjNzZjZg==` (Client ID `34a02cf8f4414e29b15921876da36f9a` : Secret `daafbccc737745039dffe53d94fc76cf`).
   - Parâmetros Form:
     - `grant_type=authorization_code`
     - `code=<authorization_code>`
     - `token_type=eg1`
   - Resposta: `{ access_token, refresh_token, account_id, displayName, expiresIn, ... }`.

3. **Geração do Exchange Code**:
   - Endpoint: `GET https://account-public-service-prod03.ol.epicgames.com/account/api/oauth/exchange`
   - Header: `Authorization: Bearer <access_token>`
   - Resposta: `{ code: "<exchange_code>" }`.

4. **Handshake de Sessão Cosmos (Cookie Jar)**:
   - `GET https://www.epicgames.com/id/api/reputation` -> Armazena o cookie `XSRF-TOKEN`.
   - `POST https://www.epicgames.com/id/api/exchange` -> JSON: `{"exchangeCode": "<exchange_code>"}`, Header: `X-XSRF-TOKEN: <valor>`.
   - `GET https://www.epicgames.com/id/api/redirect?` -> Retorna `{ "sid": "<session_id>" }`.
   - `GET https://www.unrealengine.com/id/api/set-sid?sid=<session_id>` -> Ativa a sessão no domínio `unrealengine.com`.
   - `GET https://www.unrealengine.com/api/cosmos/auth` -> Converte os cookies em tokens de autenticação JWT válidos.

5. **Verificação do EULA**:
   - `GET https://www.unrealengine.com/api/cosmos/eula/accept?eulaId=unreal_engine2&locale=en`
   - Retorna se o usuário já aceitou o contrato de licença da Unreal Engine. Se não, permite aceitar via `POST` com os mesmos parâmetros.

6. **Listagem de Blobs (Builds Pré-Compiladas)**:
   - `GET https://www.unrealengine.com/api/blobs/linux`
   - Retorna todas as versões estáveis da Unreal Engine prontas para descompactar e executar no Linux!

---

## 5. Estratégia de Implementação no Unreal Launcher

### 5.1 Eliminação da Dependência do Legendary
Em vez de depender de uma ferramenta CLI externa, o launcher passará a gerenciar a autenticação e downloads **nativamente em Rust**.
- **Vantagens**:
  - Binário 100% autocontido (não necessita de Python, pipx ou pacotes externos no sistema host).
  - Persistência segura do `refresh_token` em `~/.config/unreal-launcher/epic_session.json`.
  - Renovação silenciosa de sessão: o usuário faz login apenas uma vez; o launcher renova o token automaticamente ao iniciar.
  - Suporte completo a downloads nativos da engine que o Legendary não oferece.

### 5.2 Duas Opções de Arquitetura no Backend Rust
1. **Opção A: Integração com o crate `egs-api`**:
   - O crate `egs-api` (v0.14+) já implementa todo esse fluxo (`EpicGames::new()`, `egs.auth_code()`, `egs.cosmos_session_setup()`, `egs.engine_versions("linux")`).
   - Prós: Código amplamente testado pela comunidade do Epic Asset Manager.
   - Contras: Traz dependências adicionais no Cargo.
2. **Opção B: Implementação de Cliente HTTP Nativo (`reqwest` com cookie store)**:
   - Implementar diretamente no módulo `epic.rs` as requisições HTTP do handshake usando `reqwest` com `cookie_store(true)`.
   - Prós: Controle total sobre o fluxo, código enxuto, sem dependências desnecessárias, customização total de eventos de telemetria e progresso de download.

---

## 6. Roadmap e Futuras Implementações

### Fase 1: Autenticação Nativa Epic & Persistência de Sessão
- Substituir as chamadas de subprocesso do `legendary` por cliente HTTP nativo em Rust.
- Salvar tokens em `~/.config/unreal-launcher/epic_session.json` com permissões seguras (`0600`).
- Ao abrir o launcher, validar a sessão existente via `refresh_token`; se expirada, renovar automaticamente em background.
- Na UI, exibir avatar/nome da conta Epic logada com botão de desconectar.

### Fase 2: Catálogo de Versões da Engine na UI
- Criar uma aba ou modal "Download da Engine" na view de Engines.
- Chamar `GET /api/blobs/linux` e listar as versões disponíveis (ex: UE 5.5.4, 5.4.4, 5.3.2), indicando:
  - Número da versão.
  - Tamanho do download (ex: 34.2 GB).
  - Status: "Já Instalada", "Disponível para Download" ou "Baixando...".

### Fase 3: Gerenciador de Downloads & Extração Automática
- Implementar download em streaming com chunks em Rust, emitindo eventos de progresso:
  - Taxa de transferência (MB/s).
  - Porcentagem concluída.
  - Tempo restante estimado.
  - Suporte a cancelamento seguro.
- Ao concluir o download do `.zip`, acionar automaticamente o pipeline existente `engine::extract_engine_zip` para a pasta escolhida pelo usuário.
- Registrar a engine automaticamente na lista e sincronizar o `Install.ini`.

### Fase 4: Gerenciamento de Plugins e Marketplace (Fab)
- Acesso ao catálogo da conta Epic / Fab para download de pacotes de assets e plugins adquiridos.
- Suporte a instalação de plugins na pasta `Engine/Plugins/` ou na pasta `Plugins/` do próprio projeto.

### Fase 5: Criação de Projetos a partir de Templates
- Permitir ao usuário criar novos projetos (`Blank`, `First Person`, `Third Person`) diretamente da interface do launcher, especificando o nome, pasta de destino e se o projeto será C++ ou Blueprint, sem precisar abrir o editor previamente.

### Fase 6: Gerenciador de Compilação do Código-Fonte (Source Builds)
- Para estúdios e desenvolvedores avançados que precisam de builds customizadas da Unreal Engine:
  - Assistente para clonar repositórios do GitHub (vinculados à conta Epic).
  - Execução automatizada de `./Setup.sh`, `./GenerateProjectFiles.sh` e `make` com streaming de logs em tempo real na interface.

