# Publicação de atualizações assinadas

O ArcForge só instala automaticamente um pacote da última release oficial, via HTTPS, com assinatura Minisign válida e comentário assinado correspondente à versão e ao nome do arquivo. Não lê tokens pessoais do GitHub/gh. Sem chave incorporada ou assinatura publicada, o botão abre a página oficial de releases; a chamada de instalação também falha no backend.

## Configuração do responsável pela publicação

1. Em um computador confiável, gere a chave com `minisign -G`. Guarde a chave privada fora do repositório, com backup seguro. Nunca inclua essa chave em commits.
2. Cadastre a linha base64 da chave **pública** na variável GitHub Actions `ARCFORGE_UPDATE_PUBLIC_KEY`. Para builds locais, defina a variável de ambiente com o mesmo nome antes de executar `npm run tauri build`.
3. Cadastre o conteúdo completo do arquivo privado, codificado em base64, no secret GitHub Actions `ARCFORGE_SIGNING_KEY_BASE64`; para chaves criptografadas, cadastre a senha no secret `ARCFORGE_SIGNING_PASSWORD`; chaves sem senha usam `-W` e devem permanecer protegidas pelo cofre/secret. O publicador assina em pasta temporária privada e não imprime a chave/senha.
4. O workflow cria uma release em rascunho, testa/audita os builds, assina todos os instaladores, confere as assinaturas e só então publica. Sem as variáveis/secrets, a release permanece em rascunho.

Para assinar manualmente um pacote, execute:

```powershell
minisign -S -s C:\CaminhoSeguro\minisign.key -m ArcForge_0.1.2_x64-setup.exe -t "ArcForge version=0.1.2 asset=ArcForge_0.1.2_x64-setup.exe"
```

Publique o arquivo `.minisig` junto ao pacote, na mesma release. A versão no comentário deve ser a tag sem o prefixo `v`, e o nome deve coincidir exatamente com o asset. Pacotes antigos renomeados não passam nessa validação. Gere um instalador novo com a chave pública incorporada para iniciar a cadeia de confiança; esta correção não altera aplicativos já instalados/distribuídos.

## Sessão Epic e arquivos locais

A sessão Epic usa Windows Credential Manager no Windows e Secret Service no Linux, sem fallback de escrita em texto puro. A sessão antiga é migrada e seu JSON removido depois que o cofre confirma a gravação. Se o cofre estiver indisponível, a migração não usa o arquivo como sessão e mantém o original para uma tentativa posterior; o erro de um novo login é mostrado na UI. No Linux é necessário um serviço de cofre ativo, como GNOME Keyring ou KWallet.

As thumbnails precisam ser PNGs de até 8 MB, em `Saved/AutoScreenshot.png` de projetos monitorados. A exclusão exige descritor válido e usa a Lixeira; não exclui a raiz das pastas monitoradas. Destinos com links/junctions são recusados nas operações protegidas. ZIPs de Engine só podem ser baixados dos hosts oficiais explicitamente permitidos; um host novo da Epic exige revisão da lista, sem fallback permissivo.

## Validação e limites

Os testes de regressão cobrem origens de download, nonce, nomes reservados/traversal, escopo de projetos, thumbnails, migração/logout e assinatura adulterada. Os audits consultam npm e RustSec. A assinatura Minisign autentica o pacote para o atualizador; Authenticode do instalador Windows continua dependendo de um certificado do publicador e não é substituído por Minisign.

Não há garantia de segurança para projetos/plugins Unreal não confiáveis: o UnrealBuildTool e o Editor executam código desses projetos. A validação de caminhos não protege contra todos os ataques de um processo já controlando a mesma conta do sistema.

## Chave da versão 0.1.2

A chave pública de verificação está em `arcforge.pub` e na variável do GitHub Actions. A chave privada está cadastrada como secret do repositório por autorização do responsável. A cópia local foi protegida com DPAPI em `%USERPROFILE%\.minisign\arcforge.key.dpapi`; ela depende desta conta Windows e precisa de backup seguro antes de reinstalar ou trocar o computador. Não publique esse arquivo nem a chave privada.

