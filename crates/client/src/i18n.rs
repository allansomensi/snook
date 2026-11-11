//! Interface languages: English (the source text) and Brazilian Portuguese.
//!
//! Texts are written in English in the code and looked up with [`t`]; texts
//! with values use named placeholders, filled in by [`tw`].

use std::sync::atomic::{AtomicU8, Ordering};

use snook_core::{BallId, Difficulty, Foul, GameOverReason, Group, PocketId, ShotError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    English,
    Portuguese,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::English, Lang::Portuguese];

    /// The language's own name, as shown in the settings.
    pub fn name(self) -> &'static str {
        match self {
            Lang::English => "English",
            Lang::Portuguese => "Português (Brasil)",
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Lang::English => "en",
            Lang::Portuguese => "pt-BR",
        }
    }

    pub fn parse(s: &str) -> Option<Lang> {
        let s = s.trim().to_ascii_lowercase();
        if s.starts_with("pt") {
            Some(Lang::Portuguese)
        } else if s.starts_with("en") {
            Some(Lang::English)
        } else {
            None
        }
    }

    /// The language of the operating system, if we have a translation for it.
    pub fn system() -> Lang {
        sys_locale::get_locale().and_then(|l| Lang::parse(&l)).unwrap_or(Lang::English)
    }
}

static LANG: AtomicU8 = AtomicU8::new(0);

pub fn set_lang(lang: Lang) {
    LANG.store(lang as u8, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    if LANG.load(Ordering::Relaxed) == Lang::Portuguese as u8 {
        Lang::Portuguese
    } else {
        Lang::English
    }
}

/// Translates an interface text (returns it unchanged when there is no translation).
pub fn t(en: &str) -> &str {
    match lang() {
        Lang::English => en,
        Lang::Portuguese => portuguese(en).unwrap_or(en),
    }
}

/// Translates a text with `{name}` placeholders and fills them in.
pub fn tw(en: &str, values: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut out = t(en).to_string();
    for (key, value) in values {
        out = out.replace(&format!("{{{key}}}"), &value.to_string());
    }
    out
}

/// Translates a message that came from the server or the network (exact
/// texts, plus a few with a variable part).
pub fn server_text(text: &str) -> String {
    if lang() == Lang::English {
        return text.to_string();
    }
    if let Some(t) = portuguese(text) {
        return t.to_string();
    }
    if let Some(rest) = text.strip_prefix("You are muted for ") {
        let n = rest.split_whitespace().next().unwrap_or("?");
        return tw("You are muted for {n} more minutes.", &[("n", &n)]);
    }
    if let Some(rest) = text.strip_prefix("This account is banned for ") {
        let n = rest.split_whitespace().next().unwrap_or("?");
        let reason = rest.split("Reason: ").nth(1).unwrap_or("").trim_end_matches('.');
        return tw(
            "This account is suspended for {n} more day(s). Reason: {reason}.",
            &[("n", &n), ("reason", &reason)],
        );
    }
    if let Some(rest) = text.strip_prefix("This account is banned. Reason: ") {
        return tw(
            "This account is banned. Reason: {reason}.",
            &[("reason", &rest.trim_end_matches('.'))],
        );
    }
    for (en, key) in [
        ("You were muted for ", "You were muted: {r}"),
        ("Your account was suspended for ", "Your account was suspended: {r}"),
        ("Your account was banned: ", "Your account was banned: {r}"),
        ("You were disconnected by a moderator: ", "You were disconnected by a moderator: {r}"),
    ] {
        if let Some(rest) = text.strip_prefix(en) {
            return tw(key, &[("r", &rest)]);
        }
    }
    if let Some(rest) = text.strip_prefix("Can't shoot: ") {
        let reason = rest.trim_end_matches('.');
        return tw("Can't shoot: {e}.", &[("e", &t(reason))]);
    }
    text.to_string()
}

pub fn difficulty(d: Difficulty) -> &'static str {
    match d {
        Difficulty::Easy => t("Easy"),
        Difficulty::Medium => {
            if lang() == Lang::English {
                "Medium"
            } else {
                t("Normal")
            }
        }
        Difficulty::Hard => t("Hard"),
    }
}

pub fn group(g: Group) -> &'static str {
    t(g.name())
}

pub fn pocket(p: PocketId) -> &'static str {
    t(p.name())
}

pub fn ball(id: BallId) -> String {
    if id == BallId::CUE { t("cue ball").to_string() } else { tw("{n}-ball", &[("n", &id.0)]) }
}

/// How a game ended, naming who did it ("Ana sank the 8 too early").
pub fn result_line(r: GameOverReason, winner: &str, loser: &str) -> String {
    use GameOverReason::*;
    let (key, name) = match r {
        PocketedEight => ("{name} sank the 8-ball", winner),
        EightPocketedEarly => ("{name} sank the 8 too early", loser),
        EightInWrongPocket => ("{name} sank the 8 in the wrong pocket", loser),
        EightPocketedOnFoul => ("{name} fouled on the 8", loser),
        EightOffTable => ("{name} knocked the 8 off the table", loser),
        Resignation => ("{name} resigned", loser),
        Abandoned => ("{name} left the game", loser),
    };
    tw(key, &[("name", &name)])
}

/// How a game in the history ended, from the player's side.
pub fn history_reason(r: GameOverReason, won: bool) -> &'static str {
    use GameOverReason::*;
    t(match (r, won) {
        (PocketedEight, true) => "sank the 8-ball",
        (PocketedEight, false) => "the opponent sank the 8-ball",
        (EightPocketedEarly, true) => "the opponent sank the 8 too early",
        (EightPocketedEarly, false) => "sank the 8 too early",
        (EightInWrongPocket, true) => "the opponent sank the 8 in the wrong pocket",
        (EightInWrongPocket, false) => "sank the 8 in the wrong pocket",
        (EightPocketedOnFoul, true) => "the opponent fouled on the 8",
        (EightPocketedOnFoul, false) => "fouled on the 8",
        (EightOffTable, true) => "the opponent knocked the 8 off the table",
        (EightOffTable, false) => "knocked the 8 off the table",
        (Resignation, true) => "the opponent resigned",
        (Resignation, false) => "resigned",
        (Abandoned, true) => "the opponent left",
        (Abandoned, false) => "left the game",
    })
}

pub fn foul(f: &Foul) -> String {
    match f {
        Foul::CueBallPocketed => t("Scratch: the cue ball was pocketed").into(),
        Foul::CueBallOffTable => t("The cue ball left the table").into(),
        Foul::NoContact => t("The cue ball didn't hit any ball").into(),
        Foul::WrongBallFirst(id) => tw("Wrong ball hit first ({ball})", &[("ball", &ball(*id))]),
        Foul::NoRailAfterContact => t("No ball reached a cushion after contact").into(),
        Foul::ObjectBallOffTable(id) => tw("The {ball} left the table", &[("ball", &ball(*id))]),
        Foul::IllegalBreak => t("Illegal break").into(),
    }
}

pub fn shot_error(e: ShotError) -> String {
    t(&e.to_string()).to_string()
}

fn portuguese(en: &str) -> Option<&'static str> {
    Some(match en {
        "Right-drag · Wheel · C" => "Arraste com o botão direito · Roda · C",
        "Win %" => "% vit.",
        "Reset the camera: top view, no zoom (C)" => "Voltar a câmera: visão de cima, sem zoom (C)",
        "Drag the table to turn it · Wheel · C" => "Arraste a mesa para girar · Roda · C",
        "Drag the cue · click a ball or a spot" => "Arraste o taco · clique numa bola ou ponto",
        "{name} sank the 8-ball" => "{name} encaçapou a bola 8",
        "{name} sank the 8 too early" => "{name} encaçapou a 8 antes da hora",
        "{name} sank the 8 in the wrong pocket" => "{name} encaçapou a 8 na caçapa errada",
        "{name} fouled on the 8" => "{name} cometeu falta na 8",
        "{name} knocked the 8 off the table" => "{name} jogou a 8 para fora da mesa",
        "{name} resigned" => "{name} desistiu",
        "{name} left the game" => "{name} saiu da partida",
        "{winner} wins! ({how})" => "{winner} vence! ({how})",
        "#{n} in the world" => "#{n} no mundo",
        "3–16 letters, digits or underscores." => "3 a 16 letras, números ou sublinhados.",
        "A short line, as in rated online games." => {
            "Uma linha curta, como nas partidas online valendo rating."
        }
        "A solid club player." => "Um jogador de clube sólido.",
        "Accept friend" => "Aceitar amizade",
        "Achievement unlocked: {title}" => "Conquista desbloqueada: {title}",
        "Achievements {n}/{total}" => "Conquistas {n}/{total}",
        "Add" => "Adicionar",
        "Add a friend by username" => "Adicione um amigo pelo nome de usuário",
        "Add friend" => "Adicionar amigo",
        "Against the computer" => "Contra o computador",
        "All tournaments" => "Todos os torneios",
        "Amateur" => "Amador",
        "At least 8 characters. Use a password you don't use elsewhere." => {
            "Pelo menos 8 caracteres. Use uma senha que você não usa em outro lugar."
        }
        "At least two players are needed." => "São necessários pelo menos dois jogadores.",
        "Back" => "Voltar",
        "Black ball" => "Bola preta",
        "Block" => "Bloquear",
        "Break and run" => "Saída e limpeza",
        "Break and run 10 times." => "Faça 10 vezes a saída e limpe a mesa.",
        "Can't listen for games here: join by address below." => {
            "Não dá para procurar partidas aqui: entre pelo endereço abaixo."
        }
        "Cancel request" => "Cancelar pedido",
        "Cancel tournament" => "Cancelar torneio",
        "Casual" => "Amistoso",
        "Centurion" => "Centurião",
        "Champion" => "Campeão",
        "Champion: {name}" => "Campeão: {name}",
        "Change country" => "Mudar país",
        "Cheating or abuse" => "Trapaça ou abuso",
        "Choose…" => "Escolher…",
        "Click again to block" => "Clique de novo para bloquear",
        "Close" => "Fechar",
        "Closer" => "Finalizador",
        "Competitor" => "Competidor",
        "Connecting to the server…" => "Conectando ao servidor…",
        "Could not host a game: {e}" => "Não foi possível hospedar a partida: {e}",
        "Country" => "País",
        "Create" => "Criar",
        "Create a tournament" => "Criar um torneio",
        "Create your account" => "Crie sua conta",
        "Don't show a flag" => "Não mostrar bandeira",
        "Drag outside the table · Wheel · C" => "Arraste fora da mesa · Roda · C",
        "Dynasty" => "Dinastia",
        "Eight is enough" => "A oito basta",
        "Executioner" => "Carrasco",
        "Expert" => "Experiente",
        "Familiar face" => "Rosto conhecido",
        "Final" => "Final",
        "Finish your game first." => "Termine sua partida primeiro.",
        "First blood" => "Primeira vitória",
        "First break" => "Primeira saída",
        "Friend request sent." => "Pedido de amizade enviado.",
        "Friends" => "Amigos",
        "Games" => "Partidas",
        "Games on your network" => "Partidas na sua rede",
        "Give the tournament a name (at least 3 characters)." => {
            "Dê um nome ao torneio (pelo menos 3 caracteres)."
        }
        "Good company" => "Boa companhia",
        "Grandmaster" => "Grão-mestre",
        "Guests can only join LAN games. Create an account." => {
            "Convidados só podem jogar na rede local. Crie uma conta."
        }
        "Guests can't play rated tournaments." => {
            "Convidados não podem jogar torneios valendo rating."
        }
        "Guests don't have this." => "Convidados não têm isso.",
        "Harassment" => "Assédio",
        "Hat trick" => "Trinca",
        "Have 10 friends." => "Tenha 10 amigos.",
        "Have an account for 30 days." => "Tenha uma conta há 30 dias.",
        "Have an account for a year." => "Tenha uma conta há um ano.",
        "Host a game on this computer" => "Hospedar uma partida neste computador",
        "Host {host} · {n}/{size} players · {kind}" => {
            "Anfitrião {host} · {n}/{size} jogadores · {kind}"
        }
        "Hot hand" => "Mão quente",
        "Hustler" => "Malandro da sinuca",
        "Invalid shot." => "Tacada inválida.",
        "Join" => "Entrar",
        "Joining the LAN game…" => "Entrando na partida da rede local…",
        "Just for fun" => "Só pela diversão",
        "L" => "D",
        "LAN game" => "rede local",
        "LAN games don't change ratings." => "Partidas na rede local não mudam o rating.",
        "Legend" => "Lenda",
        "Lives at the table" => "Mora na mesa",
        "Loading the profile…" => "Carregando o perfil…",
        "Loading…" => "Carregando…",
        "Local network" => "Rede local",
        "Locked" => "Bloqueada",
        "Looking for games…" => "Procurando partidas…",
        "Make a friend." => "Faça um amigo.",
        "Master" => "Mestre",
        "Misses now and then: good for learning." => "Erra de vez em quando: bom para aprender.",
        "Name" => "Nome",
        "Names have 3–16 letters, digits or underscores." => {
            "Nomes têm de 3 a 16 letras, números ou sublinhados."
        }
        "New account" => "Nova conta",
        "New here? Choose “New account”: it takes ten seconds." => {
            "Novo por aqui? Escolha “Nova conta”: leva dez segundos."
        }
        "New tournament" => "Novo torneio",
        "No friends yet. Add someone you played with!" => {
            "Nenhum amigo ainda. Adicione alguém com quem você jogou!"
        }
        "No games yet." => "Nenhuma partida ainda.",
        "No limit" => "Sem limite",
        "No open tables. Open one and others can sit down." => {
            "Nenhuma mesa aberta. Abra uma e outros podem sentar."
        }
        "No such player." => "Jogador não encontrado.",
        "No tournaments right now. Create one!" => "Nenhum torneio agora. Crie um!",
        "Offensive name" => "Nome ofensivo",
        "Old timer" => "Veterano",
        "On a roll" => "Embalado",
        "On fire" => "Pegando fogo",
        "Only the host can start the tournament." => "Só o anfitrião pode iniciar o torneio.",
        "Open a table" => "Abrir uma mesa",
        "Open a table and wait, or invite a friend!" => {
            "Abra uma mesa e aguarde, ou convide um amigo!"
        }
        "Opponent" => "Adversário",
        "Or join by address" => "Ou entre pelo endereço",
        "Other" => "Outro",
        "Others can join at {ip}:{port}" => "Outros podem entrar em {ip}:{port}",
        "Passwords need at least 8 characters (not only digits)." => {
            "A senha precisa de pelo menos 8 caracteres (não só números)."
        }
        "Plans position and plays safeties." => "Planeja a posição e joga defensivo.",
        "Play 1,000 rated games." => "Jogue 1.000 partidas valendo rating.",
        "Play 10 rated games." => "Jogue 10 partidas valendo rating.",
        "Play 100 rated games." => "Jogue 100 partidas valendo rating.",
        "Play 25 casual games." => "Jogue 25 partidas amistosas.",
        "Play 500 rated games." => "Jogue 500 partidas valendo rating.",
        "Play in a tournament." => "Jogue um torneio.",
        "Play online" => "Jogar online",
        "Play with friends on the same network" => "Jogue com amigos na mesma rede",
        "Play your first rated game." => "Jogue sua primeira partida valendo rating.",
        "Player 1" => "Jogador 1",
        "Players" => "Jogadores",
        "Players in a tournament can't start other games." => {
            "Jogadores de um torneio não podem começar outras partidas."
        }
        "Please don't repeat the same message." => "Por favor, não repita a mesma mensagem.",
        "Pool hall legend" => "Lenda do salão",
        "Popular" => "Popular",
        "Private" => "Privada",
        "Profile" => "Perfil",
        "R: watch the last shot again" => "R: rever a última tacada",
        "REPLAY" => "REPLAY",
        "Rated" => "Valendo rating",
        "Rated games, tables, tournaments and friends" => {
            "Partidas valendo rating, mesas, torneios e amigos"
        }
        "Reach a rating of 1300." => "Alcance 1300 de rating.",
        "Reach a rating of 1500." => "Alcance 1500 de rating.",
        "Reach a rating of 1700." => "Alcance 1700 de rating.",
        "Reach a rating of 1900." => "Alcance 1900 de rating.",
        "Reach a rating of 2100." => "Alcance 2100 de rating.",
        "Recent games" => "Partidas recentes",
        "Refresh" => "Atualizar",
        "Regular" => "Frequentador",
        "Remove friend" => "Remover amigo",
        "Replay" => "Replay",
        "Report" => "Denunciar",
        "Report {name}" => "Denunciar {name}",
        "Request sent" => "Pedido enviado",
        "Rookie" => "Novato",
        "Round {n}" => "Rodada {n}",
        "Search…" => "Buscar…",
        "Semi-finals" => "Semifinais",
        "Send report" => "Enviar denúncia",
        "Shark" => "Tubarão",
        "Shows the contact point; judging the cut is up to you." => {
            "Mostra o ponto de contato; o corte fica por sua conta."
        }
        "Shows where the object ball and the cue ball go." => {
            "Mostra para onde vão a bola da vez e a branca."
        }
        "Shutout" => "Zerado",
        "Sign in" => "Entrar",
        "Sign in or create an account" => "Entre ou crie uma conta",
        "Sign out" => "Sair da conta",
        "Sit down" => "Sentar",
        "Skilled" => "Habilidoso",
        "Someone with that name is already here." => "Já tem alguém com esse nome aqui.",
        "Spam" => "Spam",
        "Start now" => "Começar agora",
        "Stop the replay (R)" => "Parar o replay (R)",
        "Table runner" => "Limpa-mesa",
        "Tables" => "Mesas",
        "Take turns on this computer" => "Revezem neste computador",
        "Thank you. The moderators will look into it." => "Obrigado. Os moderadores vão analisar.",
        "That player is not accepting invitations." => "Esse jogador não está aceitando convites.",
        "That shot was for an earlier turn." => "Essa tacada era de uma vez anterior.",
        "That table is no longer open." => "Essa mesa não está mais aberta.",
        "That tournament has already started." => "Esse torneio já começou.",
        "That tournament is full." => "Esse torneio está cheio.",
        "That tournament no longer exists." => "Esse torneio não existe mais.",
        "The first connection of the day can take up to a minute while the server wakes up." => {
            "A primeira conexão do dia pode levar até um minuto enquanto o servidor acorda."
        }
        "The first time, allow Snook through the firewall when asked." => {
            "Na primeira vez, permita o Snook no firewall quando for perguntado."
        }
        "The server could not play that shot." => "O servidor não conseguiu jogar essa tacada.",
        "The server is busy. Try again in a moment." => {
            "O servidor está ocupado. Tente de novo em instantes."
        }
        "The tournament starts when it is full (the host can start it earlier)." => {
            "O torneio começa quando lotar (o anfitrião pode começar antes)."
        }
        "There are too many tournaments right now. Join one!" => {
            "Há torneios demais agora. Entre em um!"
        }
        "This is a LAN server: join as a guest." => {
            "Este é um servidor de rede local: entre como convidado."
        }
        "Three levels, from relaxed to ruthless" => "Três níveis, do tranquilo ao implacável",
        "Tier" => "Nível",
        "Time per shot" => "Tempo por tacada",
        "Too many failed attempts. Wait a few minutes and try again." => {
            "Muitas tentativas erradas. Espere alguns minutos e tente de novo."
        }
        "Too many messages." => "Mensagens demais.",
        "Too many new accounts from this network. Try again later." => {
            "Muitas contas novas desta rede. Tente mais tarde."
        }
        "Top 100 · rated games" => "Top 100 · partidas valendo rating",
        "Top view (C)" => "Visão de cima (C)",
        "Tournament game: the next round starts from the lounge." => {
            "Partida de torneio: a próxima rodada começa pelo salão."
        }
        "Tournament games have no rematch." => "Partidas de torneio não têm revanche.",
        "Tournaments" => "Torneios",
        "Tournaments have 4, 8 or 16 players." => "Torneios têm 4, 8 ou 16 jogadores.",
        "Two players" => "Dois jogadores",
        "Unblock" => "Desbloquear",
        "Unlocked {date}" => "Desbloqueada em {date}",
        "Unstoppable" => "Imparável",
        "W" => "V",
        "Watch the last shot again (R)" => "Rever a última tacada (R)",
        "What happened? (optional)" => "O que aconteceu? (opcional)",
        "When time runs out, the opponent gets ball in hand." => {
            "Quando o tempo acaba, o adversário ganha bola na mão."
        }
        "Whitewash" => "Atropelo",
        "Win 10 games by sinking the 8-ball." => "Vença 10 partidas encaçapando a bola 8.",
        "Win 10 rated games in a row." => "Vença 10 partidas valendo rating seguidas.",
        "Win 10 rated games." => "Vença 10 partidas valendo rating.",
        "Win 10 shutouts." => "Vença 10 partidas sem o adversário encaçapar nenhuma bola.",
        "Win 100 games by sinking the 8-ball." => "Vença 100 partidas encaçapando a bola 8.",
        "Win 100 rated games." => "Vença 100 partidas valendo rating.",
        "Win 250 games by sinking the 8-ball." => "Vença 250 partidas encaçapando a bola 8.",
        "Win 3 rated games in a row." => "Vença 3 partidas valendo rating seguidas.",
        "Win 5 rated games in a row." => "Vença 5 partidas valendo rating seguidas.",
        "Win 5 tournaments." => "Vença 5 torneios.",
        "Win 50 games by sinking the 8-ball." => "Vença 50 partidas encaçapando a bola 8.",
        "Win 50 rated games." => "Vença 50 partidas valendo rating.",
        "Win 500 rated games." => "Vença 500 partidas valendo rating.",
        "Win a rated game." => "Vença uma partida valendo rating.",
        "Win a tournament." => "Vença um torneio.",
        "Win before your opponent sinks a ball of their group." => {
            "Vença antes de o adversário encaçapar uma bola do grupo dele."
        }
        "Win from the break without your opponent getting a shot." => {
            "Vença desde a saída sem o adversário dar uma tacada."
        }
        "Withdraw" => "Desistir do torneio",
        "World ranking" => "Ranking mundial",
        "Wrong table password." => "Senha da mesa incorreta.",
        "Wrong tournament password." => "Senha do torneio incorreta.",
        "You are already in a tournament." => "Você já está em um torneio.",
        "You are hosting a game" => "Você está hospedando uma partida",
        "You can't join this table." => "Você não pode entrar nesta mesa.",
        "You signed in from another place." => "Você entrou de outro lugar.",
        "You're sending invitations too quickly." => "Você está enviando convites rápido demais.",
        "You're sending messages too quickly." => "Você está enviando mensagens rápido demais.",
        "You've sent many reports. Please wait before sending more." => {
            "Você enviou muitas denúncias. Espere antes de enviar mais."
        }
        "Your friend list is full." => "Sua lista de amigos está cheia.",
        "Your next match starts by itself when both players are free." => {
            "Sua próxima partida começa sozinha quando os dois jogadores estiverem livres."
        }
        "Your profile" => "Seu perfil",
        "Your table is open" => "Sua mesa está aberta",
        "Zoom in" => "Aproximar",
        "Zoom out" => "Afastar",
        "at a table" => "em uma mesa",
        "best streak" => "melhor sequência",
        "by {host} · {n}/{size} players" => "por {host} · {n}/{size} jogadores",
        "casual games" => "partidas amistosas",
        "casual · private" => "amistosa · privada",
        "casual · public" => "amistosa · pública",
        "connecting to the server" => "conectando ao servidor",
        "eight-ball pool" => "sinuca bola 8",
        "finished" => "encerrado",
        "guest" => "convidado",
        "invites you to {kind}" => "convida você para {kind}",
        "joined today" => "entrou hoje",
        "member for {n} days" => "membro há {n} dias",
        "member for {n} months" => "membro há {n} meses",
        "member for {n} years" => "membro há {n} anos",
        "offline" => "offline",
        "open" => "inscrições",
        "optional: only friends with it can join" => "opcional: só quem tiver a senha entra",
        "password" => "senha",
        "peak {n}" => "máximo {n}",
        "playing" => "jogando",
        "rated games" => "partidas valendo rating",
        "rated · private" => "valendo rating · privada",
        "rated · public" => "valendo rating · pública",
        "request sent" => "pedido enviado",
        "the connection dropped" => "a conexão caiu",
        "to play online" => "para jogar online",
        "tournament" => "torneio",
        "tournaments won" => "torneios vencidos",
        "waiting for an opponent…" => "aguardando um adversário…",
        "win rate" => "aproveitamento",
        "wins" => "vitórias",
        "won on the 8" => "vitórias na bola 8",
        "{host}'s game" => "Partida de {host}",
        "{name} wants to be friends" => "{name} quer ser seu amigo",
        "{name} wants to be your friend." => "{name} quer ser seu amigo.",
        "{n} online" => "{n} online",
        "sank the 8-ball" => "encaçapou a bola 8",
        "the opponent sank the 8-ball" => "o adversário encaçapou a bola 8",
        "the opponent sank the 8 too early" => "o adversário encaçapou a 8 antes da hora",
        "sank the 8 too early" => "encaçapou a 8 antes da hora",
        "the opponent sank the 8 in the wrong pocket" => {
            "o adversário encaçapou a 8 na caçapa errada"
        }
        "sank the 8 in the wrong pocket" => "encaçapou a 8 na caçapa errada",
        "the opponent fouled on the 8" => "o adversário cometeu falta na 8",
        "fouled on the 8" => "cometeu falta na 8",
        "the opponent knocked the 8 off the table" => "o adversário jogou a 8 para fora da mesa",
        "knocked the 8 off the table" => "jogou a 8 para fora da mesa",
        "the opponent resigned" => "o adversário desistiu",
        "resigned" => "desistiu",
        "the opponent left" => "o adversário saiu",
        "left the game" => "saiu da partida",
        "You are muted for {n} more minutes." => "Você está silenciado por mais {n} minutos.",
        "This account is suspended for {n} more day(s). Reason: {reason}." => {
            "Esta conta está suspensa por mais {n} dia(s). Motivo: {reason}."
        }
        "This account is banned. Reason: {reason}." => "Esta conta está banida. Motivo: {reason}.",
        "You were muted: {r}" => "Você foi silenciado: {r}",
        "Your account was suspended: {r}" => "Sua conta foi suspensa: {r}",
        "Your account was banned: {r}" => "Sua conta foi banida: {r}",
        "You were disconnected by a moderator: {r}" => {
            "Você foi desconectado por um moderador: {r}"
        }
        "You were disconnected by a moderator." => "Você foi desconectado por um moderador.",
        "Please sign in again." => "Por favor, entre novamente.",
        "Your password was changed. Sign in again." => "Sua senha foi alterada. Entre novamente.",
        "Accept rematch" => "Aceitar revanche",
        "Accept" => "Aceitar",
        "Aim" => "Mirar",
        "Aiming guide" => "Guia de mira",
        "Anti-aliasing (smooth edges)" => "Anti-serrilhado (bordas suaves)",
        "Back to lounge" => "Voltar ao salão",
        "Back to menu" => "Voltar ao menu",
        "Balanced: smooth on most laptops." => "Equilibrado: fluido na maioria dos notebooks.",
        "Ball in hand" => "Bola na mão",
        "Break the rack" => "Abra a mesa",
        "Camera pulls back to watch each shot" => "Câmera se afasta para mostrar cada tacada",
        "Camera" => "Câmera",
        "Cancel" => "Cancelar",
        "Casual game: ratings unchanged" => "Partida amistosa: o rating não muda",
        "Center" => "Centro",
        "Chat" => "Chat",
        "Click a ball or a spot" => "Clique numa bola ou num ponto",
        "Click to chat, Enter to send" => "Clique para conversar, Enter envia",
        "Controls (H)" => "Controles (H)",
        "Controls" => "Controles",
        "Create account" => "Criar conta",
        "Cue ball contact" => "Contato na bola branca",
        "Decline" => "Recusar",
        "Done" => "Pronto",
        "Drag on the table: cue follows" => "Arraste na mesa: o taco segue",
        "Drag the cue ball" => "Arraste a bola branca",
        "Elevation" => "Elevação",
        "Enter the lounge" => "Entrar no salão",
        "Everything, with the sharpest balls: for gaming PCs." => {
            "Tudo no máximo, bolas mais nítidas: para PCs gamer."
        }
        "Fine aim" => "Mira fina",
        "Finish this match first." => "Termine esta partida primeiro.",
        "For old or very slow computers." => "Para computadores antigos ou muito lentos.",
        "Full screen (F11)" => "Tela cheia (F11)",
        "Full" => "Completo",
        "Game server" => "Servidor",
        "Graphics quality" => "Qualidade gráfica",
        "High" => "Alta",
        "Hold Space, release" => "Segure Espaço e solte",
        "Invite" => "Convidar",
        "Leave to the lounge" => "Sair para o salão",
        "Leave" => "Sair",
        "Leaving now counts as a loss." => "Sair agora conta como derrota.",
        "Log in" => "Entrar",
        "Log out" => "Sair da conta",
        "Lounge chat" => "Chat do salão",
        "Low" => "Baixa",
        "Main menu" => "Menu principal",
        "Medium" => "Média",
        "Menu / help" => "Menu / ajuda",
        "New here? Pick a name and password, then Create account." => {
            "Novo aqui? Escolha um nome e uma senha e clique em Criar conta."
        }
        "Next game starts in a moment…" => "A próxima partida começa em instantes…",
        "No rated games yet. Be the first!" => {
            "Nenhuma partida valendo rating ainda. Seja o primeiro!"
        }
        "Nobody else is here right now." => "Ninguém mais por aqui agora.",
        "Off" => "Desligado",
        "On the 8-ball: call a pocket" => "Na bola 8: cante a caçapa",
        "On the 8-ball" => "Na bola 8",
        "Online lounge" => "Salão online",
        "Open table" => "Mesa aberta",
        "Password" => "Senha",
        "Passwords need at least 6 characters." => "A senha precisa ter pelo menos 6 caracteres.",
        "Paused" => "Pausado",
        "Play against the computer" => "Jogar contra o computador",
        "Play people around the world and climb the ranking." => {
            "Jogue com gente do mundo todo e suba no ranking."
        }
        "Play two players" => "Jogar em dois",
        "Play" => "Jogar",
        "Player 2" => "Jogador 2",
        "Player" => "Jogador",
        "Players online" => "Jogadores online",
        "Quick aim" => "Mira rápida",
        "Quit" => "Sair",
        "Rated games (affect your rating)" => "Partidas valendo rating",
        "Reconnected." => "Reconectado.",
        "Rematch" => "Revanche",
        "Resign this game" => "Desistir desta partida",
        "Restart the game to apply this change." => "Reinicie o jogo para aplicar esta mudança.",
        "Resume" => "Continuar",
        "Retry" => "Tentar de novo",
        "Right-drag · Wheel · Tab · C" => "Botão direito · Roda · Tab · C",
        "Saving the result…" => "Salvando o resultado…",
        "Say hello… (Enter to send)" => "Diga oi… (Enter envia)",
        "Say hi in the chat, or invite a friend!" => "Diga oi no chat ou convide um amigo!",
        "Send" => "Enviar",
        "Settings" => "Configurações",
        "Shift + drag · A / D" => "Shift + arrastar · A / D",
        "Shoot" => "Tacada",
        "Shooting…" => "Tacando…",
        "Short" => "Curto",
        "Show the controls panel" => "Mostrar o painel de controles",
        "Signing in…" => "Entrando…",
        "Soft shadows from every lamp, contact shading." => {
            "Sombras suaves de cada lâmpada e sombra de contato."
        }
        "Solids (1–7)" => "Lisas (1–7)",
        "Spin ball · ↑ ↓ · Q / E" => "Bola de efeito · ↑ ↓ · Q / E",
        "Spin" => "Efeito",
        "Standard" => "Padrão",
        "Stay" => "Ficar",
        "Stripes (9–15)" => "Listradas (9–15)",
        "The game goes on while this is open." => "A partida continua com este menu aberto.",
        "Top players" => "Melhores jogadores",
        "Two players on this computer" => "Dois jogadores neste computador",
        "Use default" => "Usar padrão",
        "Username" => "Usuário",
        "Usernames have 3–16 letters, digits or underscores." => {
            "O usuário deve ter de 3 a 16 letras, números ou _."
        }
        "W / S (jump & massé)" => "W / S (pulo e massé)",
        "Watch demo" => "Ver demonstração",
        "Won" => "Vitórias",
        "You signed in from another window or computer." => {
            "Você entrou em outra janela ou computador."
        }
        "You win!" => "Você venceu!",
        "Your name" => "Seu nome",
        "Your opponent went back to the lounge." => "Seu adversário voltou ao salão.",
        "Your turn" => "Sua vez",
        "any ball" => "qualquer bola",
        "draw" => "puxada",
        "follow" => "seguida",
        "stun" => "seca",
        "solids" => "lisas",
        "stripes" => "listradas",
        "eight-ball  ·  play, chat, climb the ranking" => {
            "sinuca bola 8  ·  jogue, converse, suba no ranking"
        }
        "lounge" => "salão",
        "release Space to shoot · Esc cancels" => "solte Espaço para tacar · Esc cancela",
        " · left english" => " · efeito à esquerda",
        " · right english" => " · efeito à direita",
        "Language" => "Idioma",
        "Graphics" => "Gráficos",
        "Game" => "Jogo",
        "Sound" => "Som",
        "Reset to defaults" => "Restaurar padrões",
        "Online" => "Online",
        "Menu" => "Menu",
        "Rating" => "Rating",
        "Ultra" => "Ultra",
        "Esc · H" => "Esc · H",
        "Call the pocket for the 8-ball" => "Cante a caçapa da bola 8",
        "Click a pocket on the table, or pick one here:" => {
            "Clique numa caçapa na mesa ou escolha aqui:"
        }
        "Called pocket" => "Caçapa cantada",
        "Change" => "Trocar",
        "Sinking the 8 in any other pocket loses the game." => {
            "Encaçapar a 8 em outra caçapa perde a partida."
        }
        "Mouse aim sensitivity" => "Sensibilidade da mira (mouse)",
        "Vignette (cinematic edges)" => "Vinheta (bordas cinematográficas)",
        "Show FPS" => "Mostrar FPS",
        "Call a pocket for the 8-ball first: click a pocket." => {
            "Cante a caçapa da bola 8 primeiro: clique numa caçapa."
        }
        "Reconnecting…" => "Reconectando…",
        "the game is over" => "a partida acabou",
        "the cue ball cannot be placed there" => "a bola branca não pode ficar aí",
        "call a pocket for the 8-ball" => "cante a caçapa da bola 8",
        "That username is taken." => "Esse nome de usuário já existe.",
        "Wrong username or password." => "Usuário ou senha incorretos.",
        "Your session expired. Please log in again." => "Sua sessão expirou. Entre novamente.",
        "This version of Snook is out of date. Please update the game." => {
            "Esta versão do Snook está desatualizada. Atualize o jogo."
        }
        "It's not your turn." => "Não é a sua vez.",
        "That player is not online." => "Esse jogador não está online.",
        "That player is in a game." => "Esse jogador está em uma partida.",
        "You can't invite yourself." => "Você não pode convidar a si mesmo.",
        "That invitation is no longer valid." => "Esse convite não é mais válido.",
        "That player is no longer available." => "Esse jogador não está mais disponível.",
        "You've played many rated games against each other today: this one is casual." => {
            "Vocês já jogaram muitas partidas valendo rating hoje: esta será amistosa."
        }
        "Resign first to leave a game in progress." => {
            "Desista primeiro para sair de uma partida em andamento."
        }
        "Malformed message." => "Mensagem inválida.",
        "Please log in first." => "Entre primeiro.",
        "Could not create the account." => "Não foi possível criar a conta.",
        "The cue ball can't be placed there." => "A bola branca não pode ficar aí.",
        "Disconnected." => "Desconectado.",
        "Lost contact with the server." => "Perdeu o contato com o servidor.",
        "The server closed the connection." => "O servidor encerrou a conexão.",
        "The server took too long to answer." => "O servidor demorou demais para responder.",
        "The server is not answering (connection refused)." => {
            "O servidor não está respondendo (conexão recusada)."
        }
        "The server address is not valid (example: wss://snook.example.com/ws)." => {
            "O endereço do servidor não é válido (exemplo: wss://snook.exemplo.com/ws)."
        }
        "The server address has no host name." => "O endereço do servidor não tem nome de host.",
        "Welcome to Snook! Be kind, play fair." => "Bem-vindo ao Snook! Seja gentil, jogue limpo.",
        "Ball in hand: drag the cue ball{zone} · release to aim" => {
            "Bola na mão: arraste a branca{zone} · solte para mirar"
        }
        " behind the line" => " atrás da linha",
        "Ball in hand  ·  {group}" => "Bola na mão  ·  {group}",
        "Can't shoot: {e}." => "Não dá para tacar: {e}.",
        "Chat ({n})" => "Chat ({n})",
        "Computer ({level})" => "Computador ({level})",
        "Connecting to {server}…" => "Conectando a {server}…",
        "Connection lost. Reconnecting (attempt {n})…" => {
            "Conexão perdida. Reconectando (tentativa {n})…"
        }
        "Connection lost. Reconnecting…" => "Conexão perdida. Reconectando…",
        "Elevation {deg}°" => "Elevação {deg}°",
        "Power {pct}%" => "Força {pct}%",
        "Foul by {shooter}: {foul}." => "Falta de {shooter}: {foul}.",
        "Graphics set to {q} for smooth play. You can change this in Settings." => {
            "Gráficos ajustados para {q} para o jogo ficar liso. Dá para mudar em Configurações."
        }
        "Rating {r} ({d})" => "Rating {r} ({d})",
        "Server: {s}" => "Servidor: {s}",
        "The {ball} was spotted." => "A {ball} voltou para a mesa.",
        "Waiting for {to} ({kind})…" => "Aguardando {to} ({kind})…",
        "rated" => "valendo rating",
        "casual" => "amistosa",
        "Waiting for {name}…" => "Aguardando {name}…",
        "{by} declined your invitation." => "{by} recusou seu convite.",
        "{from} invites you to {kind}" => "{from} convida você para {kind}",
        "a rated game" => "uma partida valendo rating",
        "a casual game" => "uma partida amistosa",
        "{name} calls the 8-ball in the {pocket} pocket." => {
            "{name} canta a bola 8 na caçapa {pocket}."
        }
        "{name} has ball in hand…" => "{name} está com bola na mão…",
        "{name} is aiming…" => "{name} está mirando…",
        "{name} is on the 8: click a pocket to call it" => {
            "{name} está na bola 8: clique numa caçapa para cantá-la"
        }
        "{name} is thinking…" => "{name} está pensando…",
        "{name} left the game." => "{name} saiu da partida.",
        "{name} resigned." => "{name} desistiu.",
        "{name} to break" => "{name} abre a mesa",
        "{name} to shoot ({target})" => "Vez de {name} ({target})",
        "{name}'s turn" => "Vez de {name}",
        "{next} has ball in hand{where}." => "{next} está com bola na mão{where}.",
        " behind the head string" => " atrás da linha de saída",
        "{opponent} is back." => "{opponent} voltou.",
        "{opponent} lost the connection. Waiting up to {secs} s…" => {
            "{opponent} perdeu a conexão. Aguardando até {secs} s…"
        }
        "{opponent} wants a rematch." => "{opponent} quer revanche.",
        "{opponent} went back to the lounge." => "{opponent} voltou ao salão.",
        "{shooter} pocketed {balls}." => "{shooter} encaçapou {balls}.",
        "{shooter} takes {group}." => "{shooter} fica com as {group}.",
        "{who} ran out of time: ball in hand for the opponent." => {
            "O tempo de {who} acabou: bola na mão para o adversário."
        }
        "{winner} wins: {reason}!" => "{winner} vence: {reason}!",
        "{name}  ·  rating {r}  ·  {g} games, {w} wins" => {
            "{name}  ·  rating {r}  ·  {g} partidas, {w} vitórias"
        }
        "{name} wants a rematch!" => "{name} quer revanche!",
        "{name} wins!" => "{name} vence!",
        "{name} wins" => "{name} vence",
        "{r} · playing" => "{r} · jogando",
        "{r} · {g} games" => "{r} · {g} partidas",
        "…and {n} more" => "…e mais {n}",
        "Sound volume  {v}%" => "Volume do som  {v}%",
        "Could not find the server “{host}”." => "Servidor “{host}” não encontrado.",
        "Network error: {e}." => "Erro de rede: {e}.",
        "Secure connection failed: {e}." => "Falha na conexão segura: {e}.",
        "The server refused the connection (HTTP {code})." => {
            "O servidor recusou a conexão (HTTP {code})."
        }
        "Connection error: {e}." => "Erro de conexão: {e}.",
        "Called: {pocket}" => "Cantada: {pocket}",
        "{name} calls the {pocket} pocket." => "{name} canta a caçapa {pocket}.",
        "cue ball" => "bola branca",
        "{n}-ball" => "bola {n}",
        "Scratch: the cue ball was pocketed" => "A branca caiu na caçapa",
        "The cue ball left the table" => "A branca saiu da mesa",
        "The cue ball didn't hit any ball" => "A branca não tocou em nenhuma bola",
        "Wrong ball hit first ({ball})" => "Tocou primeiro na bola errada ({ball})",
        "No ball reached a cushion after contact" => "Nenhuma bola tocou uma tabela após o contato",
        "The {ball} left the table" => "A {ball} saiu da mesa",
        "Illegal break" => "Saída irregular",
        "pocketed the 8-ball" => "encaçapou a bola 8",
        "opponent pocketed the 8-ball too early" => "o adversário encaçapou a bola 8 antes da hora",
        "opponent sank the 8-ball in the wrong pocket" => {
            "o adversário encaçapou a bola 8 na caçapa errada"
        }
        "opponent fouled on the 8-ball" => "o adversário cometeu falta na bola 8",
        "opponent knocked the 8-ball off the table" => {
            "o adversário jogou a bola 8 para fora da mesa"
        }
        "opponent resigned" => "o adversário desistiu",
        "opponent left the game" => "o adversário saiu da partida",
        "head left corner" => "canto esquerdo da cabeceira",
        "head right corner" => "canto direito da cabeceira",
        "left side" => "meio esquerda",
        "right side" => "meio direita",
        "foot left corner" => "canto esquerdo do pé",
        "foot right corner" => "canto direito do pé",
        "Easy" => "Fácil",
        "Hard" => "Difícil",
        "Normal" => "Médio",
        "Power" => "Força",
        "Drag down to pull the cue back, release to shoot" => {
            "Arraste para baixo para puxar o taco e solte para tacar"
        }
        "Drag the power bar down, release" => "Arraste a barra de força e solte",
        "Top view" => "Vista de cima",
        "3D view" => "Vista 3D",
        "Reset camera" => "Centralizar câmera",
        "No messages yet." => "Nenhuma mensagem ainda.",
        "▼ newer messages" => "▼ mensagens novas",
        "Right click or Esc cancels" => "Botão direito ou Esc cancela",
        "Camera follows the cue" => "Câmera acompanha o taco",
        "Pull back for shots" => "Afastar nas tacadas",
        "Drag the cue around the ball" => "Arraste o taco em volta da bola",
        "Drag the cue · click a spot" => "Arraste o taco · clique num ponto",
        "Drag up for more power, release to shoot" => {
            "Arraste para cima para mais força e solte para tacar"
        }
        "Drag the power bar up, release" => "Arraste a barra de força para cima e solte",
        "Time's up: {name}'s shot was played at full power." => {
            "Tempo esgotado: a tacada de {name} foi feita com força máxima."
        }
        "Dark room (spotlight on the table)" => "Ambiente escuro (luz só na mesa)",
        "Hide controls" => "Ocultar controles",
        "Drag right for more power, release to shoot" => {
            "Arraste para a direita para mais força e solte para tacar"
        }
        "Drag the power bar right, release" => "Arraste a barra de força para a direita e solte",
        "Interface size  {v}%" => "Tamanho da interface  {v}%",
        "Drag to set the power" => "Arraste para ajustar a força",
        "Set the power, then press Shoot" => "Ajuste a força e aperte Tacar",
        "Shoot!" => "Jogar",
        "Set the power, then press the green button" => "Ajuste a força e aperte o botão verde",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_and_fills_placeholders() {
        set_lang(Lang::Portuguese);
        assert_eq!(t("Play"), "Jogar");
        assert_eq!(t("not in the table"), "not in the table");
        assert_eq!(tw("{name} wins!", &[("name", &"Ana")]), "Ana vence!");
        assert_eq!(ball(BallId(8)), "bola 8");
        assert_eq!(
            server_text("Can't shoot: the game is over."),
            "Não dá para tacar: a partida acabou."
        );
        set_lang(Lang::English);
        assert_eq!(tw("{name} wins!", &[("name", &"Ana")]), "Ana wins!");
    }
}
