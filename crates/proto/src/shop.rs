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
}

impl Category {
    pub const ALL: [Category; 4] =
        [Category::Cloth, Category::Cue, Category::Chalk, Category::Balls];

    pub fn code(self) -> &'static str {
        match self {
            Category::Cloth => "cloth",
            Category::Cue => "cue",
            Category::Chalk => "chalk",
            Category::Balls => "balls",
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
        }
    }
}

/// How an item looks (the game draws it; colours are sRGB).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    /// Cloth: the base colour of the felt.
    Cloth([u8; 3]),
    /// Cue: butt, forearm accent and ring colours.
    Cue { butt: [u8; 3], accent: [u8; 3], ring: [u8; 3] },
    /// Chalk: the colour on the cue tip.
    Chalk([u8; 3]),
    /// Balls: a palette for the seven colours (1–7 / 9–15), the 8 and the
    /// cue ball, and the ivory of stripes and number circles.
    Balls { colors: [[u8; 3]; 7], eight: [u8; 3], ivory: [u8; 3] },
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
) -> Item {
    Item { code, category: Category::Cue, name, price, look: Look::Cue { butt, accent, ring } }
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
) -> Item {
    Item {
        code,
        category: Category::Balls,
        name,
        price,
        look: Look::Balls { colors, eight, ivory },
    }
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

/// Every item, in the order the shop shows them.
pub const ITEMS: &[Item] = &[
    // Cloth.
    cloth("cloth_green", "Tournament green", Price::Free, [24, 96, 48]),
    cloth("cloth_blue", "Electric blue", Price::Coins(400), [16, 54, 112]),
    cloth("cloth_burgundy", "Burgundy", Price::Coins(400), [76, 13, 22]),
    cloth("cloth_charcoal", "Charcoal", Price::Coins(600), [32, 34, 38]),
    cloth("cloth_teal", "Teal", Price::Coins(600), [10, 78, 80]),
    cloth("cloth_purple", "Royal purple", Price::Coins(900), [50, 22, 84]),
    cloth("cloth_camel", "Camel", Price::Coins(900), [116, 84, 44]),
    cloth("cloth_midnight", "Midnight", Price::Vip, [12, 17, 40]),
    cloth("cloth_champion", "Champion's red", Price::Achievement("tournament_win"), [110, 12, 16]),
    // Cues.
    cue("cue_classic", "Classic", Price::Free, [35, 20, 12], [20, 20, 24], [240, 238, 228]),
    cue("cue_rosewood", "Rosewood", Price::Coins(500), [92, 34, 22], [40, 18, 12], [200, 170, 110]),
    cue(
        "cue_ebony",
        "Ebony & silver",
        Price::Coins(800),
        [14, 12, 12],
        [60, 60, 66],
        [200, 205, 215],
    ),
    cue("cue_carbon", "Carbon", Price::Coins(1200), [30, 32, 36], [200, 30, 30], [30, 32, 36]),
    cue(
        "cue_ivory",
        "Ivory inlay",
        Price::Coins(1500),
        [235, 225, 200],
        [35, 20, 12],
        [190, 150, 70],
    ),
    cue("cue_royal", "Crown", Price::Vip, [24, 30, 80], [190, 150, 60], [230, 200, 110]),
    cue(
        "cue_legend",
        "Legend",
        Price::Achievement("rating_1900"),
        [20, 14, 10],
        [210, 170, 60],
        [240, 210, 120],
    ),
    // Chalk.
    chalk("chalk_blue", "Blue chalk", Price::Free, [40, 80, 150]),
    chalk("chalk_green", "Green chalk", Price::Coins(150), [40, 130, 70]),
    chalk("chalk_red", "Red chalk", Price::Coins(150), [170, 40, 40]),
    chalk("chalk_purple", "Purple chalk", Price::Coins(200), [110, 50, 160]),
    chalk("chalk_gold", "Gold chalk", Price::Vip, [210, 170, 60]),
    // Balls.
    balls("balls_classic", "Classic", Price::Free, CLASSIC, [22, 22, 24], [246, 242, 228]),
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
    ),
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
