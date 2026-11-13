//! The shop: cosmetic items (cloth, cue, chalk, balls), coin packs and the
//! VIP subscription. Shared by the server (which owns the wallets and sells)
//! and the game (which shows and draws the items).
//!
//! Every category has a free default that everyone owns. Other items are
//! bought with coins, come with an achievement, or are part of VIP.

/// What an item changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    Cloth,
    Cue,
    Chalk,
    Balls,
    /// The frame around the player's flag and name card.
    Frame,
    /// The colour of the name in the lounge (VIP).
    Name,
    /// What happens when this player wins on the 8.
    Effect,
}

impl Category {
    pub const ALL: [Category; 7] = [
        Category::Cloth,
        Category::Cue,
        Category::Chalk,
        Category::Balls,
        Category::Frame,
        Category::Name,
        Category::Effect,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Category::Cloth => "cloth",
            Category::Cue => "cue",
            Category::Chalk => "chalk",
            Category::Balls => "balls",
            Category::Frame => "frame",
            Category::Name => "name",
            Category::Effect => "effect",
        }
    }

    pub fn parse(s: &str) -> Option<Category> {
        Category::ALL.into_iter().find(|c| c.code() == s)
    }

    /// The item everyone has.
    pub fn default_item(self) -> &'static str {
        match self {
            Category::Cloth => "cloth_green",
            Category::Cue => "cue_classic",
            Category::Chalk => "chalk_blue",
            Category::Balls => "balls_classic",
            Category::Frame => "frame_none",
            Category::Name => "name_tier",
            Category::Effect => "effect_none",
        }
    }
}

/// The pattern of a cue's forearm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CueStyle {
    /// Plain wood with rings.
    Plain,
    /// Pointed inlays ("points") of the accent colour into the forearm.
    Points,
    /// A spiral wrap of the accent colour.
    Spiral,
    /// A woven carbon pattern.
    Weave,
}

/// The finish of a ball set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BallFinish {
    /// Phenolic resin: deep colour, bright clear coat.
    Gloss,
    /// A pearly sheen that shifts with the light.
    Pearl,
    /// Brushed metal.
    Metal,
    /// Old ivory: soft and warm.
    Matte,
}

/// What a winning 8-ball does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectKind {
    None,
    /// A burst of sparks from the pocket.
    Sparks,
    /// Confetti over the table.
    Confetti,
    /// Rings of light spreading over the cloth.
    Shockwave,
    /// Flames licking up from the pocket.
    Flames,
    /// A shower of coins.
    Coins,
    /// Fireworks above the table.
    Fireworks,
}

/// How an item looks (the game draws it; colours are sRGB).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    /// Cloth: the base colour of the felt.
    Cloth([u8; 3]),
    /// Cue: butt, forearm accent and ring colours, and the forearm pattern.
    Cue {
        butt: [u8; 3],
        accent: [u8; 3],
        ring: [u8; 3],
        style: CueStyle,
    },
    /// Chalk: the colour on the cue tip.
    Chalk([u8; 3]),
    /// Balls: a palette for the seven colours (1–7 / 9–15), the 8 and the
    /// cue ball, and the ivory of stripes and number circles.
    Balls {
        colors: [[u8; 3]; 7],
        eight: [u8; 3],
        ivory: [u8; 3],
        finish: BallFinish,
    },
    /// Frame: two colours (a gradient) and whether it shines.
    Frame {
        a: [u8; 3],
        b: [u8; 3],
        shine: bool,
    },
    /// Name: the colour of the name (`None` is the tier's colour), and a
    /// second colour for a gradient.
    Name(Option<([u8; 3], [u8; 3])>),
    Effect {
        kind: EffectKind,
        color: [u8; 3],
    },
}

/// How an item is obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Price {
    /// Everyone has it.
    Free,
    Coins(u32),
    /// Comes with this achievement.
    Achievement(&'static str),
    /// Only for VIP members (while they are VIP).
    Vip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Item {
    pub code: &'static str,
    pub category: Category,
    /// English name (translated by the game).
    pub name: &'static str,
    pub price: Price,
    pub look: Look,
}

impl Item {
    /// Can be bought with coins (and so given to a friend).
    pub fn coins(&self) -> Option<u32> {
        match self.price {
            Price::Coins(n) => Some(n),
            _ => None,
        }
    }
}

const fn cloth(code: &'static str, name: &'static str, price: Price, c: [u8; 3]) -> Item {
    Item { code, category: Category::Cloth, name, price, look: Look::Cloth(c) }
}

const fn cue(
    code: &'static str,
    name: &'static str,
    price: Price,
    butt: [u8; 3],
    accent: [u8; 3],
    ring: [u8; 3],
    style: CueStyle,
) -> Item {
    Item {
        code,
        category: Category::Cue,
        name,
        price,
        look: Look::Cue { butt, accent, ring, style },
    }
}

const fn chalk(code: &'static str, name: &'static str, price: Price, c: [u8; 3]) -> Item {
    Item { code, category: Category::Chalk, name, price, look: Look::Chalk(c) }
}

const fn balls(
    code: &'static str,
    name: &'static str,
    price: Price,
    colors: [[u8; 3]; 7],
    eight: [u8; 3],
    ivory: [u8; 3],
    finish: BallFinish,
) -> Item {
    Item {
        code,
        category: Category::Balls,
        name,
        price,
        look: Look::Balls { colors, eight, ivory, finish },
    }
}

const fn frame(
    code: &'static str,
    name: &'static str,
    price: Price,
    a: [u8; 3],
    b: [u8; 3],
    shine: bool,
) -> Item {
    Item { code, category: Category::Frame, name, price, look: Look::Frame { a, b, shine } }
}

const fn name_style(code: &'static str, name: &'static str, a: [u8; 3], b: [u8; 3]) -> Item {
    Item { code, category: Category::Name, name, price: Price::Vip, look: Look::Name(Some((a, b))) }
}

const fn effect(
    code: &'static str,
    name: &'static str,
    price: Price,
    kind: EffectKind,
    color: [u8; 3],
) -> Item {
    Item { code, category: Category::Effect, name, price, look: Look::Effect { kind, color } }
}

const CLASSIC: [[u8; 3]; 7] = [
    [255, 204, 10],
    [18, 58, 205],
    [228, 28, 24],
    [108, 36, 168],
    [255, 118, 12],
    [12, 142, 58],
    [142, 26, 28],
];

const IVORY: [u8; 3] = [246, 242, 228];

use BallFinish::{Gloss, Matte, Metal, Pearl};
use CueStyle::{Plain, Points, Spiral, Weave};

/// Every item, in the order the shop shows them.
pub const ITEMS: &[Item] = &[
    // Cloth.
    cloth("cloth_green", "Tournament green", Price::Free, [24, 96, 48]),
    cloth("cloth_blue", "Electric blue", Price::Coins(400), [16, 54, 112]),
    cloth("cloth_burgundy", "Burgundy", Price::Coins(400), [76, 13, 22]),
    cloth("cloth_olive", "Olive", Price::Coins(400), [62, 72, 30]),
    cloth("cloth_navy", "Navy", Price::Coins(500), [14, 28, 64]),
    cloth("cloth_charcoal", "Charcoal", Price::Coins(600), [32, 34, 38]),
    cloth("cloth_teal", "Teal", Price::Coins(600), [10, 78, 80]),
    cloth("cloth_forest", "Forest", Price::Coins(600), [18, 58, 34]),
    cloth("cloth_slate", "Slate grey", Price::Coins(700), [58, 66, 74]),
    cloth("cloth_wine", "Red wine", Price::Coins(700), [96, 18, 40]),
    cloth("cloth_purple", "Royal purple", Price::Coins(900), [50, 22, 84]),
    cloth("cloth_camel", "Camel", Price::Coins(900), [116, 84, 44]),
    cloth("cloth_orange", "Burnt orange", Price::Coins(900), [150, 62, 18]),
    cloth("cloth_rose", "Dusty rose", Price::Coins(1000), [138, 70, 82]),
    cloth("cloth_sand", "Sand", Price::Coins(1000), [150, 128, 92]),
    cloth("cloth_ice", "Ice blue", Price::Coins(1200), [70, 118, 150]),
    cloth("cloth_lime", "Lime", Price::Coins(1200), [92, 138, 24]),
    cloth("cloth_black", "Black tie", Price::Coins(1500), [16, 16, 18]),
    cloth("cloth_cobalt", "Cobalt", Price::Coins(1500), [20, 64, 168]),
    cloth("cloth_midnight", "Midnight", Price::Vip, [12, 17, 40]),
    cloth("cloth_emerald", "Emerald", Price::Vip, [6, 110, 74]),
    cloth("cloth_champion", "Champion's red", Price::Achievement("tournament_win"), [110, 12, 16]),
    // Cues.
    cue("cue_classic", "Classic", Price::Free, [35, 20, 12], [20, 20, 24], [240, 238, 228], Plain),
    cue(
        "cue_maple",
        "Natural maple",
        Price::Coins(300),
        [196, 150, 96],
        [120, 80, 40],
        [30, 24, 20],
        Plain,
    ),
    cue(
        "cue_rosewood",
        "Rosewood",
        Price::Coins(500),
        [92, 34, 22],
        [40, 18, 12],
        [200, 170, 110],
        Points,
    ),
    cue(
        "cue_walnut",
        "Walnut",
        Price::Coins(600),
        [70, 44, 26],
        [190, 160, 110],
        [210, 200, 180],
        Points,
    ),
    cue(
        "cue_ebony",
        "Ebony & silver",
        Price::Coins(800),
        [14, 12, 12],
        [60, 60, 66],
        [200, 205, 215],
        Plain,
    ),
    cue(
        "cue_zebra",
        "Zebrawood",
        Price::Coins(900),
        [150, 110, 60],
        [40, 26, 16],
        [230, 220, 200],
        Spiral,
    ),
    cue(
        "cue_bocote",
        "Bocote",
        Price::Coins(1000),
        [140, 96, 44],
        [30, 20, 12],
        [200, 160, 80],
        Points,
    ),
    cue(
        "cue_carbon",
        "Carbon",
        Price::Coins(1200),
        [30, 32, 36],
        [200, 30, 30],
        [30, 32, 36],
        Weave,
    ),
    cue(
        "cue_ocean",
        "Deep ocean",
        Price::Coins(1200),
        [16, 40, 70],
        [60, 170, 200],
        [220, 230, 240],
        Spiral,
    ),
    cue(
        "cue_cherry",
        "Cherry",
        Price::Coins(1200),
        [120, 26, 30],
        [240, 230, 210],
        [200, 170, 110],
        Points,
    ),
    cue(
        "cue_jade",
        "Jade",
        Price::Coins(1400),
        [24, 80, 60],
        [200, 230, 210],
        [190, 150, 70],
        Points,
    ),
    cue(
        "cue_ivory",
        "Ivory inlay",
        Price::Coins(1500),
        [235, 225, 200],
        [35, 20, 12],
        [190, 150, 70],
        Points,
    ),
    cue(
        "cue_tiger",
        "Tiger",
        Price::Coins(1600),
        [200, 120, 30],
        [20, 16, 12],
        [20, 16, 12],
        Spiral,
    ),
    cue(
        "cue_amethyst",
        "Amethyst",
        Price::Coins(1800),
        [60, 24, 90],
        [200, 170, 240],
        [220, 220, 230],
        Points,
    ),
    cue(
        "cue_arctic",
        "Arctic carbon",
        Price::Coins(2000),
        [210, 220, 228],
        [60, 140, 220],
        [40, 44, 50],
        Weave,
    ),
    cue(
        "cue_inferno",
        "Inferno",
        Price::Coins(2400),
        [24, 10, 8],
        [240, 90, 20],
        [250, 200, 60],
        Spiral,
    ),
    cue("cue_royal", "Crown", Price::Vip, [24, 30, 80], [190, 150, 60], [230, 200, 110], Points),
    cue(
        "cue_obsidian",
        "Obsidian gold",
        Price::Vip,
        [10, 10, 12],
        [210, 170, 70],
        [230, 200, 110],
        Weave,
    ),
    cue(
        "cue_legend",
        "Legend",
        Price::Achievement("rating_1900"),
        [20, 14, 10],
        [210, 170, 60],
        [240, 210, 120],
        Points,
    ),
    // Chalk.
    chalk("chalk_blue", "Blue chalk", Price::Free, [40, 80, 150]),
    chalk("chalk_green", "Green chalk", Price::Coins(150), [40, 130, 70]),
    chalk("chalk_red", "Red chalk", Price::Coins(150), [170, 40, 40]),
    chalk("chalk_tan", "Tan chalk", Price::Coins(150), [180, 150, 100]),
    chalk("chalk_grey", "Grey chalk", Price::Coins(150), [120, 124, 130]),
    chalk("chalk_purple", "Purple chalk", Price::Coins(200), [110, 50, 160]),
    chalk("chalk_orange", "Orange chalk", Price::Coins(200), [220, 110, 30]),
    chalk("chalk_pink", "Pink chalk", Price::Coins(200), [220, 110, 160]),
    chalk("chalk_teal", "Teal chalk", Price::Coins(200), [30, 150, 150]),
    chalk("chalk_yellow", "Yellow chalk", Price::Coins(250), [230, 200, 50]),
    chalk("chalk_black", "Black chalk", Price::Coins(300), [28, 28, 32]),
    chalk("chalk_white", "White chalk", Price::Coins(300), [235, 235, 230]),
    chalk("chalk_gold", "Gold chalk", Price::Vip, [210, 170, 60]),
    chalk("chalk_silver", "Silver chalk", Price::Vip, [190, 196, 205]),
    // Balls.
    balls("balls_classic", "Classic", Price::Free, CLASSIC, [22, 22, 24], IVORY, Gloss),
    balls(
        "balls_tv",
        "TV set",
        Price::Coins(700),
        [
            [255, 214, 0],
            [0, 90, 220],
            [235, 30, 30],
            [140, 50, 200],
            [255, 130, 0],
            [0, 160, 70],
            [150, 30, 30],
        ],
        [16, 16, 18],
        [252, 250, 244],
        Gloss,
    ),
    balls(
        "balls_vintage",
        "Vintage",
        Price::Coins(900),
        [
            [214, 170, 40],
            [40, 64, 130],
            [170, 48, 34],
            [96, 50, 110],
            [200, 110, 40],
            [40, 106, 60],
            [110, 40, 32],
        ],
        [30, 26, 24],
        [236, 222, 190],
        Matte,
    ),
    balls(
        "balls_earth",
        "Earth tones",
        Price::Coins(1000),
        [
            [196, 160, 70],
            [60, 80, 100],
            [150, 70, 50],
            [100, 70, 90],
            [180, 110, 60],
            [80, 110, 70],
            [120, 70, 50],
        ],
        [36, 30, 26],
        [238, 228, 205],
        Matte,
    ),
    balls(
        "balls_pastel",
        "Pastel",
        Price::Coins(1200),
        [
            [250, 220, 110],
            [120, 160, 230],
            [240, 120, 120],
            [180, 140, 220],
            [250, 170, 110],
            [120, 200, 150],
            [200, 120, 130],
        ],
        [40, 40, 48],
        [250, 248, 240],
        Gloss,
    ),
    balls(
        "balls_ocean",
        "Ocean",
        Price::Coins(1200),
        [
            [120, 220, 230],
            [10, 50, 140],
            [0, 150, 180],
            [40, 80, 170],
            [80, 200, 210],
            [10, 110, 120],
            [20, 60, 100],
        ],
        [8, 20, 40],
        [240, 250, 252],
        Gloss,
    ),
    balls(
        "balls_sunset",
        "Sunset",
        Price::Coins(1300),
        [
            [255, 200, 60],
            [140, 40, 110],
            [240, 70, 50],
            [110, 30, 90],
            [255, 130, 40],
            [200, 90, 70],
            [160, 30, 50],
        ],
        [40, 14, 30],
        [255, 244, 228],
        Gloss,
    ),
    balls(
        "balls_forest",
        "Forest",
        Price::Coins(1300),
        [
            [200, 190, 80],
            [40, 90, 70],
            [130, 60, 30],
            [70, 60, 40],
            [170, 110, 40],
            [30, 110, 50],
            [90, 40, 30],
        ],
        [20, 26, 18],
        [236, 232, 214],
        Matte,
    ),
    balls(
        "balls_neon",
        "Neon",
        Price::Coins(1500),
        [
            [230, 255, 0],
            [0, 200, 255],
            [255, 30, 90],
            [190, 60, 255],
            [255, 150, 0],
            [0, 240, 120],
            [255, 60, 30],
        ],
        [10, 10, 12],
        [250, 250, 250],
        Gloss,
    ),
    balls(
        "balls_candy",
        "Candy",
        Price::Coins(1500),
        [
            [255, 230, 120],
            [120, 200, 255],
            [255, 110, 150],
            [210, 150, 255],
            [255, 180, 120],
            [140, 240, 180],
            [255, 130, 130],
        ],
        [90, 50, 110],
        [255, 252, 250],
        Pearl,
    ),
    balls(
        "balls_pearl",
        "Pearl",
        Price::Coins(2000),
        CLASSIC,
        [30, 30, 36],
        [248, 246, 240],
        Pearl,
    ),
    balls(
        "balls_mono",
        "Monochrome",
        Price::Coins(2000),
        [
            [230, 230, 230],
            [60, 60, 66],
            [180, 180, 186],
            [40, 40, 44],
            [210, 210, 214],
            [90, 90, 96],
            [140, 140, 146],
        ],
        [12, 12, 14],
        [250, 250, 250],
        Gloss,
    ),
    balls(
        "balls_chrome",
        "Chrome",
        Price::Coins(2500),
        [
            [230, 190, 60],
            [70, 110, 190],
            [200, 70, 60],
            [140, 90, 170],
            [220, 140, 70],
            [70, 160, 110],
            [160, 80, 70],
        ],
        [40, 42, 46],
        [222, 226, 232],
        Metal,
    ),
    balls(
        "balls_royal",
        "Jewels",
        Price::Vip,
        [
            [230, 180, 40],
            [20, 40, 140],
            [160, 10, 30],
            [80, 20, 120],
            [210, 100, 20],
            [10, 100, 60],
            [120, 16, 30],
        ],
        [16, 14, 20],
        [245, 236, 210],
        Pearl,
    ),
    balls(
        "balls_gold",
        "Gold rush",
        Price::Vip,
        [
            [240, 200, 70],
            [180, 140, 50],
            [210, 160, 60],
            [150, 110, 40],
            [230, 180, 80],
            [170, 130, 50],
            [130, 90, 30],
        ],
        [20, 16, 10],
        [250, 240, 210],
        Metal,
    ),
    // Frames.
    frame("frame_none", "No frame", Price::Free, [0, 0, 0], [0, 0, 0], false),
    frame("frame_bronze", "Bronze", Price::Coins(300), [176, 112, 60], [110, 66, 34], false),
    frame("frame_silver", "Silver", Price::Coins(600), [214, 218, 226], [130, 136, 146], false),
    frame("frame_gold", "Gold", Price::Coins(1000), [240, 200, 90], [170, 120, 40], true),
    frame("frame_felt", "Green felt", Price::Coins(400), [40, 140, 80], [16, 70, 40], false),
    frame("frame_wood", "Mahogany", Price::Coins(500), [130, 60, 34], [70, 30, 16], false),
    frame("frame_ice", "Ice", Price::Coins(900), [190, 230, 250], [90, 160, 210], true),
    frame("frame_fire", "Fire", Price::Coins(1200), [255, 180, 40], [210, 40, 20], true),
    frame("frame_neon", "Neon", Price::Coins(1200), [0, 240, 220], [220, 40, 240], true),
    frame("frame_diamond", "Diamond", Price::Coins(2500), [230, 250, 255], [140, 200, 255], true),
    frame("frame_crown", "VIP crown", Price::Vip, [250, 214, 110], [190, 130, 40], true),
    frame("frame_amethyst", "Amethyst", Price::Vip, [200, 150, 255], [100, 40, 170], true),
    frame(
        "frame_champion",
        "Champion",
        Price::Achievement("tournament_win"),
        [250, 220, 120],
        [170, 20, 30],
        true,
    ),
    // Name styles (VIP).
    Item {
        code: "name_tier",
        category: Category::Name,
        name: "Tier colour",
        price: Price::Free,
        look: Look::Name(None),
    },
    name_style("name_gold", "Gold", [252, 216, 120], [214, 160, 60]),
    name_style("name_rose", "Rose gold", [250, 190, 170], [210, 120, 110]),
    name_style("name_aqua", "Aqua", [140, 240, 240], [60, 170, 230]),
    name_style("name_violet", "Violet", [220, 170, 255], [150, 90, 240]),
    // Orange: red is for administrators' names alone.
    name_style("name_ember", "Ember", [255, 210, 100], [245, 140, 40]),
    name_style("name_mint", "Mint", [170, 250, 200], [70, 210, 150]),
    name_style("name_frost", "Frost", [240, 248, 255], [160, 200, 240]),
    // Winning effects.
    effect("effect_none", "No effect", Price::Free, EffectKind::None, [0, 0, 0]),
    effect("effect_sparks", "Sparks", Price::Coins(500), EffectKind::Sparks, [255, 210, 120]),
    effect("effect_confetti", "Confetti", Price::Coins(800), EffectKind::Confetti, [255, 255, 255]),
    effect(
        "effect_shockwave",
        "Shockwave",
        Price::Coins(1000),
        EffectKind::Shockwave,
        [120, 220, 255],
    ),
    effect("effect_coins", "Coin shower", Price::Coins(1200), EffectKind::Coins, [240, 200, 80]),
    effect("effect_flames", "Flames", Price::Coins(1500), EffectKind::Flames, [255, 120, 30]),
    effect("effect_fireworks", "Fireworks", Price::Vip, EffectKind::Fireworks, [255, 220, 140]),
    effect("effect_royal", "Royal sparks", Price::Vip, EffectKind::Sparks, [200, 150, 255]),
];

pub fn item(code: &str) -> Option<&'static Item> {
    ITEMS.iter().find(|i| i.code == code)
}

/// A pack of coins sold for real money (the price is set on the payment
/// provider and shown by the server).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoinPack {
    pub code: &'static str,
    pub coins: u32,
    /// Extra coins included (shown as a bonus).
    pub bonus: u32,
}

pub const COIN_PACKS: &[CoinPack] = &[
    CoinPack { code: "coins_500", coins: 500, bonus: 0 },
    CoinPack { code: "coins_1200", coins: 1200, bonus: 100 },
    CoinPack { code: "coins_2800", coins: 2800, bonus: 400 },
    CoinPack { code: "coins_7000", coins: 7000, bonus: 1500 },
];

pub fn coin_pack(code: &str) -> Option<&'static CoinPack> {
    COIN_PACKS.iter().find(|p| p.code == code)
}

/// The VIP subscription's product code, and the coins each paid month brings.
pub const VIP_PLAN: &str = "vip_monthly";
pub const VIP_MONTHLY_COINS: u32 = 400;

/// Whether a player owns or may use `item`.
pub fn can_use(item: &Item, owned: &[String], achievements: &[String], vip: bool) -> bool {
    match item.price {
        Price::Free => true,
        Price::Coins(_) => owned.iter().any(|o| o == item.code),
        Price::Achievement(a) => achievements.iter().any(|c| c == a),
        Price::Vip => vip,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_is_consistent() {
        let mut codes: Vec<_> = ITEMS.iter().map(|i| i.code).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), ITEMS.len());
        for c in Category::ALL {
            let d = item(c.default_item()).expect("default exists");
            assert_eq!(d.category, c);
            assert_eq!(d.price, Price::Free);
            assert!(d.code.starts_with(c.code()));
        }
        for i in ITEMS {
            assert!(i.code.starts_with(i.category.code()), "{}", i.code);
            if let Price::Achievement(a) = i.price {
                assert!(crate::ACHIEVEMENTS.iter().any(|x| x.code == a), "{a}");
            }
        }
        let none: Vec<String> = Vec::new();
        assert!(can_use(item("cloth_green").unwrap(), &none, &none, false));
        assert!(!can_use(item("cloth_blue").unwrap(), &none, &none, false));
        assert!(can_use(item("cloth_blue").unwrap(), &["cloth_blue".into()], &none, false));
        assert!(can_use(item("cue_royal").unwrap(), &none, &none, true));
    }
}
