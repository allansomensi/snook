# Colocando o servidor do Snook no ar (Render + Neon, grátis)

Este guia leva o servidor do zero até o jogo conectando nele, usando apenas
planos gratuitos. Leva uns 20 minutos.

```
 Jogadores ──wss──▶  Render (servidor Snook, Docker)  ──TLS──▶  Neon (PostgreSQL)
                         └─ /admin  (painel da equipe)
```

- **Render** roda o servidor (o mesmo binário de sempre, em um container).
- **Neon** guarda contas, ratings, histórico, conquistas, amizades e denúncias.
- **Vercel não é necessária**: o painel administrativo é servido pelo próprio
  servidor em `/admin` (mesma origem, sem CORS, um deploy só). Se um dia você
  quiser um site do jogo, a Vercel é ótima para isso, e o servidor já expõe
  `GET /api/leaderboard` e `GET /api/status` (JSON público, com CORS) para o site
  mostrar o ranking e quantos estão online.

## O que saber sobre os planos grátis

| | Limite | O que significa para o jogo |
|---|---|---|
| Render Free | dorme após 15 min sem tráfego; acorda em ~30–60 s | A primeira conexão do dia demora. O jogo avisa ("o servidor está acordando") e tenta de novo sozinho. Use o workflow *keep-alive* (passo 7) para evitar. |
| Render Free | 750 h/mês por conta | Dá para 1 serviço ligado 24/7 (744 h). Não crie um segundo serviço grátis ligado. |
| Render Free | reinicia a cada deploy | Partidas em andamento são perdidas no deploy. Faça deploys em horários calmos. |
| Render Free | 512 MB RAM, CPU compartilhada | Suficiente para dezenas de jogadores simultâneos. A simulação das tacadas roda fora do lock principal. |
| Neon Free | 0,5 GB, suspende após 5 min parado | Centenas de milhares de partidas cabem. Acordar o banco leva <1 s. |

Quando lançar de verdade: Render Starter (US$ 7/mês, não dorme) resolve os
três primeiros itens sem mudar nada no código.

## 1. Banco de dados no Neon

1. Crie uma conta em <https://neon.tech> e um projeto (região **AWS US East
   (N. Virginia)**, a mesma do Render no passo 2, para a menor latência).
2. Em **Dashboard → Connection Details**, escolha o banco `neondb`, papel
   padrão, e **desmarque "Connection pooling"** (use a conexão **direta**, sem
   `-pooler` no host: o servidor mantém poucas conexões fixas e usa
   *prepared statements*).
3. Copie a string. Ela tem este formato:

   ```
   postgresql://usuario:senha@ep-xxxx-123456.us-east-1.aws.neon.tech/neondb?sslmode=require
   ```

Não precisa criar tabelas: o servidor aplica as migrations sozinho ao iniciar
(`crates/server/migrations`).

## 2. Servidor no Render

1. Crie uma conta em <https://render.com> e conecte seu GitHub.
2. **New → Blueprint**, escolha o repositório `snook`. O Render lê o
   `render.yaml` da raiz e propõe o serviço `snook-server` (Docker, plano Free,
   região Virginia, health check em `/health`).
3. Ele vai pedir as variáveis marcadas como secretas:
   - `DATABASE_URL`: a string do Neon do passo 1.
   - `SNOOK_ADMINS`: seu nome de usuário no jogo (ex.: `allan`). Vários: `allan,maria`.
4. **Apply**. O primeiro build (Rust em release) leva ~10–15 min no plano
   grátis; os seguintes usam cache.
5. Quando ficar "Live", teste:

   ```bash
   curl https://snook-server.onrender.com/health
   ```

   Deve responder `ok`. (O nome exato da URL aparece no topo do serviço.)

As outras variáveis já vêm no `render.yaml`:

| Variável | Valor | Para quê |
|---|---|---|
| `SNOOK_CLIENT_IP_HEADER` | `cf-connecting-ip,true-client-ip,x-forwarded-for` | O Render fica atrás da Cloudflare: o servidor lê o IP real do jogador desses cabeçalhos. Sem isso todos parecem vir do mesmo IP e os limites anti-abuso (contas por IP, conexões por IP) bloqueariam gente inocente. O painel avisa se isso acontecer. |
| `SNOOK_DB_POOL` | `5` | Conexões com o banco (o Neon grátis aceita bem). |
| `SNOOK_SHOT_CLOCK` | `30` | Segundos por tacada nas partidas online. |
| `SNOOK_MOTD` | (opcional) | Mensagem de boas-vindas do salão. |
| `RUST_LOG` | `info` | Nível dos logs (veja em **Logs** no Render). |

> Alternativa sem Blueprint: **New → Web Service → Existing image**, usando a
> imagem `ghcr.io/allansomensi/snook-server:latest` publicada pelo workflow de
> release (passo 6). O deploy fica instantâneo porque não compila nada no Render.

## 3. Virar administrador

1. Abra o jogo, **Jogar online → Nova conta**, crie a conta com o nome que pôs
   em `SNOOK_ADMINS` (ou entre, se ela já existe). Ao entrar, ela vira admin.
2. Abra `https://snook-server.onrender.com/admin` e entre com o mesmo usuário e
   senha.

No painel: visão geral (online, partidas, contas, denúncias abertas), busca de
jogadores, ban temporário/permanente, silenciar, desconectar, encerrar todas
as sessões, mudar papel (jogador/moderador/admin), redefinir senha, fila de
denúncias com o trecho de chat anexado, partidas e torneios ao vivo, anúncio
no salão e o log de auditoria de toda ação da equipe. Moderadores podem
moderar; só administradores mudam papéis e senhas. Veja `docs/ADMIN.md`.

## 4. Apontar o jogo para o servidor

O endereço do jogo é a URL do Render com `wss://` e `/ws` no fim:

```
wss://snook-server.onrender.com/ws
```

- **Para testar agora**: no jogo, **Configurações → Jogo → Servidor**, cole o
  endereço.
- **Para distribuir**: compile com o endereço embutido, assim ninguém precisa
  configurar nada:

  ```bash
  SNOOK_SERVER=wss://snook-server.onrender.com/ws cargo build --release -p snook
  ```

  No Windows (PowerShell):

  ```powershell
  $env:SNOOK_SERVER="wss://snook-server.onrender.com/ws"; cargo build --release -p snook
  ```

## 5. Rede local (LAN)

Não precisa de servidor nenhum: em **Rede local**, um jogador clica em
**Hospedar uma partida neste computador** e os outros veem a partida na lista
(descoberta automática por broadcast UDP na porta 47878) ou entram pelo IP
mostrado na tela (porta 47879). Na primeira vez o Windows pergunta sobre o
firewall: permita em **redes privadas**. Na LAN não há contas nem rating; há
salão, chat, mesas, torneios e partidas como no online.

## 6. Publicar versões (GitHub Actions)

1. No GitHub: **Settings → Secrets and variables → Actions → Variables → New
   variable**: `SNOOK_SERVER` = `wss://snook-server.onrender.com/ws`.
2. Crie uma tag:

   ```bash
   git tag v0.1.0
   ```

   ```bash
   git push origin v0.1.0
   ```

3. O workflow **Release** compila o jogo para Windows, macOS e Linux com o
   servidor embutido e anexa os arquivos à página de Releases, e publica a
   imagem do servidor em `ghcr.io/<você>/snook-server`.

## 7. (Opcional) Manter o servidor acordado

Defina a variável de repositório `SNOOK_HEALTH_URL` =
`https://snook-server.onrender.com/health`. O workflow **Keep the server
awake** chama o servidor a cada 10 minutos. O GitHub às vezes atrasa execuções
agendadas, então ainda pode haver um "acordar" ocasional; no plano pago isso
some.

## 8. Backups

O Neon grátis mantém histórico de 24 h (restauração a um ponto no tempo pelo
painel). Para uma cópia sua:

```bash
pg_dump "postgresql://usuario:senha@ep-xxxx.us-east-1.aws.neon.tech/neondb?sslmode=require" -Fc -f snook-$(date +%F).dump
```

## Problemas comuns

| Sintoma | Causa provável |
|---|---|
| Jogo fica em "Conectando ao servidor…" ~1 min e entra | Instância grátis acordando. Normal. |
| `could not start: … password authentication failed` nos logs | `DATABASE_URL` errada. Copie de novo do Neon. |
| `… prepared statement … already exists` | Usou a string com `-pooler`. Use a conexão direta. |
| Jogadores recebem "Muitas contas novas desta rede" sem motivo | `SNOOK_CLIENT_IP_HEADER` ausente. O painel mostra o aviso "Every player comes from the same address". |
| "Esta versão do Snook está desatualizada" | Jogo e servidor com protocolos diferentes. Atualize os dois juntos. |
| Partida em andamento sumiu | Deploy ou o Render reiniciou a instância (o salão e as partidas ficam na memória; contas e resultados ficam no banco). |
