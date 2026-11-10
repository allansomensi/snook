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

pub fn reason(r: GameOverReason) -> &'static str {
    t(r.description())
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
        "Casual game: ratings unchanged" => "Partida casual: o rating não muda",
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
        "No rated games yet. Be the first!" => "Nenhuma partida ranqueada ainda. Seja o primeiro!",
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
        "Rated games (affect your rating)" => "Partidas ranqueadas (valem rating)",
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
            "Vocês já jogaram muitas partidas ranqueadas hoje: esta será casual."
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
        "rated" => "ranqueada",
        "casual" => "casual",
        "Waiting for {name}…" => "Aguardando {name}…",
        "{by} declined your invitation." => "{by} recusou seu convite.",
        "{from} invites you to {kind}" => "{from} convida você para {kind}",
        "a rated game" => "uma partida ranqueada",
        "a casual game" => "uma partida casual",
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
