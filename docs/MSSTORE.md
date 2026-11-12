# Microsoft Store

O jogo vai para a Microsoft Store como um pacote **MSIX**, feito do mesmo
código das outras versões. A Store assina o pacote, instala e atualiza
sozinha. O instalador `.msi` e os arquivos do GitHub continuam como estão.

## O que muda na versão da Store

O build da Store usa a feature `store` do jogo (`crates/client/src/channel.rs`):

- **Sem compras com dinheiro real pelo Stripe.** A política da Store pede que
  jogos vendam itens digitais pelo sistema de compras da própria Store. Até
  essa integração existir (etapa 2), a aba Moedas explica que as moedas vêm de
  jogar (partidas valendo rating e o bônus diário) e o VIP não é oferecido.
  Os itens da loja continuam compráveis com moedas. Quem já é VIP continua
  vendo até quando, mas sem o botão de gerenciar a assinatura.
- **Runtime do C embutido** (`+crt-static`): o `.exe` não depende do Visual
  C++ Redistributable.
- **Firewall da rede local já liberado** pelo pacote (portas 47879/TCP e
  47878/UDP, só em redes privadas e de domínio): hospedar uma partida na rede
  local não mostra o aviso do Windows.

| Arquivo | O que é |
|---|---|
| `crates/client/msix/AppxManifest.xml` | o manifesto do pacote (identidade, ícones, firewall) |
| `crates/client/msix/Assets/` | os ícones do pacote (`cargo test --release -p snook export_msix_assets -- --ignored` os refaz a partir do emblema) |
| `tools/msix/make-msix.ps1` | preenche o manifesto e monta o `.msix` |
| `.github/workflows/msstore.yml` | gera o MSIX e o envia à Store |
| `crates/server/pages/` | a política de privacidade (`/privacidade` e `/privacy`) |

## Uma vez só: preparar a Store

### 1. Reservar o nome e anotar a identidade

No [Partner Center](https://partner.microsoft.com/dashboard): **Apps and
games → New product → MSIX or PWA app**, reserve o nome. Depois, em
**Product management → Product identity**, anote os valores e crie as
variáveis do repositório (GitHub: **Settings → Secrets and variables →
Actions → Variables**):

| Variável | Valor do Partner Center |
|---|---|
| `MSSTORE_IDENTITY_NAME` | `Package/Identity/Name` |
| `MSSTORE_PUBLISHER` | `Package/Identity/Publisher` (começa com `CN=`) |
| `MSSTORE_PUBLISHER_DISPLAY_NAME` | `Package/Properties/PublisherDisplayName` |
| `MSSTORE_DISPLAY_NAME` | o nome reservado |
| `MSSTORE_PRODUCT_ID` | o **Store ID** (`9...`, em Product identity) |

### 2. Política de privacidade

O servidor publica a política em `https://<seu-servidor>/privacidade`
(português) e `/privacy` (inglês). Antes de usá-la, **leia o texto** em
`crates/server/pages/` e ajuste o que for preciso. Ele descreve o que o jogo
guarda hoje, mas não é uma revisão jurídica.

Na Render (**Environment**), defina:

| Variável | Exemplo |
|---|---|
| `SNOOK_GAME_NAME` | o nome do jogo na Store |
| `SNOOK_PRIVACY_CONTROLLER` | seu nome ou o da sua empresa |
| `SNOOK_PRIVACY_CONTACT` | um e-mail para pedidos de privacidade (sem ele, a página aponta para as *issues* do GitHub) |

No Partner Center, use `https://<seu-servidor>/privacy` como **Privacy policy
URL**.

### 3. Servidor sempre acordado

No plano gratuito da Render o servidor dorme e o primeiro jogador espera
cerca de um minuto. Defina a variável do repositório `SNOOK_HEALTH_URL` como
`https://<seu-servidor>/health` (o workflow *Keep the server awake* passa a
acordá-lo a cada 10 minutos) ou use um plano pago.

### 4. Primeiro envio (pelo site)

A primeira submissão precisa ser feita no Partner Center, porque ela inclui a
página da loja, a classificação etária e o preço:

1. Gere o pacote: **Actions → Microsoft Store → Run workflow**. Com as
   variáveis do passo 1 o pacote já sai com a identidade certa. Baixe o
   artefato `msix` da execução.
2. **Start your submission** e preencha:
   - **Pricing and availability:** grátis; mercados.
   - **Properties:** categoria *Games*; marque que o jogo tem chat e jogo
     online; requisitos: placa de vídeo com OpenGL.
   - **Age ratings:** responda o questionário (IARC). Diga que há
     **interação entre usuários (chat)**, **compartilhamento de nome de
     usuário** e **itens compráveis com moeda do jogo**.
   - **Packages:** envie o `.msix` baixado.
   - **Store listings** (português e inglês): descrição, capturas de tela
     (as de `docs/screenshots/` servem; o mínimo é 1366×768) e o link de
     suporte.
3. Envie para certificação (até três dias úteis).

### 5. Envio automático das próximas versões

Para o workflow enviar sozinho, o GitHub precisa de acesso à sua conta:

1. Em [Microsoft Entra ID](https://entra.microsoft.com): **App registrations →
   New registration** (nome livre, sem URL). Anote o **Application (client)
   ID** e o **Directory (tenant) ID**; em **Certificates & secrets**, crie um
   *client secret* e anote o valor.
2. No Partner Center: **Account settings → User management → Microsoft Entra
   applications → Add**, escolha esse aplicativo, papel **Manager**. O
   **Seller ID** está em **Account settings → Legal info → Developer**.
3. No GitHub, crie os *secrets* (**Settings → Secrets and variables → Actions
   → Secrets**): `MSSTORE_TENANT_ID`, `MSSTORE_CLIENT_ID`,
   `MSSTORE_CLIENT_SECRET`, `MSSTORE_SELLER_ID`.

## Lançar uma versão

Do mesmo jeito de sempre (veja `RELEASING.md`): versão no `Cargo.toml`,
seção no `CHANGELOG.md`, tag `vX.Y.Z`. O workflow *Microsoft Store* gera o
MSIX (a versão `0.1.2` vira `0.1.2.0`, como a Store exige) e cria uma
**submissão em rascunho** com ele. Confira no Partner Center e clique em
**Submit to the Store**. Com a variável `MSSTORE_AUTO_SUBMIT` = `true` o
envio para certificação é direto.

Betas (`v0.2.0-beta.1`) não vão para a Store.

## Testar o pacote no seu computador

Com o **Modo de desenvolvedor** ligado (Configurações → Sistema → Para
desenvolvedores), no PowerShell, na pasta do projeto:

```powershell
$env:RUSTFLAGS = "-C target-feature=+crt-static"
cargo build --release --locked -p snook --features store
./tools/msix/make-msix.ps1 -IdentityName Snookered.Teste -Publisher "CN=Teste" `
  -PublisherDisplayName "Teste" -DisplayName "Snookered (teste)" -Register
```

O jogo aparece no menu Iniciar como "Snookered (teste)". Para remover:
**Configurações → Aplicativos → Snookered (teste) → Desinstalar**.

## O nome

O jogo se chama **Snookered**. O nome que o jogador vê vem de uma constante só,
`snook_proto::GAME_NAME` (janela, menu, salão, notificações, mensagens do
servidor, páginas web); o instalador `.msi` (`crates/client/wix/main.wxs`),
os metadados do `.exe` (`crates/client/build.rs`) e os textos traduzidos
(`crates/client/lang/`) repetem o nome e mudam junto. Os nomes internos
continuam `snook` (crates, `snook.exe`, a pasta de configurações, o
identificador da rede local), para que atualizações e logins existentes
continuem funcionando.

Na Store, o nome vem das variáveis `MSSTORE_DISPLAY_NAME` (GitHub) e
`SNOOK_GAME_NAME` (Render, para a política de privacidade); use **Snookered**
nas duas.
