//! The nine-node Sherwood skill tree (issue #122).
//!
//! This module is the single source of truth for the tree's shape, prices,
//! prerequisites, effect values and purchase rule.  The perks panel renders
//! [`SkillTreeRow`]s built here, and the authoritative campaign purchase asks
//! [`skill_purchase_state`] — the two cannot drift apart.
//!
//! Storage is deliberately one additive, `#[serde(default)]` bitmask on
//! [`crate::campaign::PcDescription`] rather than five new booleans.  The four
//! perks that predate the tree stay on `PcStatus` as a compatibility mirror:
//! [`PcDescription::has_skill`] reads either home and `grant_skill` writes both.
//! A pre-tree save therefore loads without a migration, while the one bitcode
//! layout change is covered by a single version bump for replay, network and
//! native snapshots.

use serde::{Deserialize, Serialize};

use crate::pc_status::{PcStatus, Perk};

// ─── Effect constants ────────────────────────────────────────────

/// Fleet-footed multiplies a PC's walk and run `speed_factor` by this value.
pub const FLEET_FOOTED_SPEED_FACTOR: f32 = 1.25;
/// Hardy grants this many life points on top of `LIFEPOINTS_PC`.  The base
/// constant itself is unchanged, so heals still clamp at the original cap.
pub const HARDY_BONUS_LIFE_POINTS: i16 = 25;
/// Silent Step scales walking noise by this value; running noise is unchanged.
pub const SILENT_STEP_NOISE_MULTIPLIER: f32 = 0.5;
/// Keen Ear raises the Listen radius from the authored 750 to this value.
pub const KEEN_EAR_LISTEN_RADIUS: i32 = 1050;
/// Cost of buying every node: four Tier-I at one, four Tier-II and the single
/// Tier-III at two.  The story corpus grants 18 clovers, leaving four spare.
pub const TOTAL_TREE_COST: i32 = 4 * 1 + 4 * 2 + 1 * 2;

// ─── Node ────────────────────────────────────────────────────────

/// One unlockable node.  The discriminants are part of the bitmask layout and
/// of the snapshot encoding; append only, never reorder.
#[repr(u8)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum SkillNode {
    /// Tier I — move faster on foot.
    FleetFooted = 0,
    /// Tier I — survive more blows.
    Hardy = 1,
    /// Tier I — walk without being heard.
    SilentStep = 2,
    /// Tier I — hear guards from further away.
    KeenEar = 3,
    /// Tier II — move orders survive a swordfight.
    FightThrough = 4,
    /// Tier II — a friendly strike deals no damage.
    CarefulBlade = 5,
    /// Tier II — lockpick and climb authorization.
    BurglarsKit = 6,
    /// Tier II — a knock-out takes the victim's money.
    LootTheKnockout = 7,
    /// Tier III — a friendly strike does not push its target.
    SteadyBlade = 8,
}

/// The gameplay behaviour a node grants.  Kept as data so effect hooks can be
/// tested without a mission and so the panel can name tactical uses rather
/// than raw stat deltas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SkillEffect {
    SpeedFactor(f32),
    BonusLifePoints(i16),
    WalkNoiseMultiplier(f32),
    ListenRadius(i32),
    FightThrough,
    CarefulBlade,
    LockpickAndClimb,
    AutoLoot,
    SteadyBlade,
}

impl SkillNode {
    /// Every node, in panel order: tiers first, then tactical identities.
    pub const ALL: [Self; 9] = [
        Self::FleetFooted,
        Self::Hardy,
        Self::SilentStep,
        Self::KeenEar,
        Self::FightThrough,
        Self::CarefulBlade,
        Self::BurglarsKit,
        Self::LootTheKnockout,
        Self::SteadyBlade,
    ];

    /// The three tiers are 1-based so the panel can print I / II / III.
    pub const fn tier(self) -> u8 {
        match self {
            Self::FleetFooted | Self::Hardy | Self::SilentStep | Self::KeenEar => 1,
            Self::FightThrough | Self::CarefulBlade | Self::BurglarsKit | Self::LootTheKnockout => {
                2
            }
            Self::SteadyBlade => 3,
        }
    }

    /// Tier I costs one clover; Tiers II and III cost two.  This is the only
    /// price table in the tree: no panel, command or hook may hardcode a cost.
    pub const fn cost(self) -> i32 {
        if self.tier() == 1 { 1 } else { 2 }
    }

    /// Nodes that must be owned before this one can be bought.
    ///
    /// The issue names exactly one dependency set: all four Tier-II nodes gate
    /// Steady Blade.  No other prerequisite is invented here.
    pub const fn prerequisites(self) -> &'static [Self] {
        match self {
            Self::SteadyBlade => &[
                Self::FightThrough,
                Self::CarefulBlade,
                Self::BurglarsKit,
                Self::LootTheKnockout,
            ],
            _ => &[],
        }
    }

    /// The original flat perk this node supersedes, if any.  Kept so old saves
    /// and the existing purchase command keep working during integration.
    pub const fn legacy_perk(self) -> Option<Perk> {
        match self {
            Self::FightThrough => Some(Perk::KeepMovingInCombat),
            Self::CarefulBlade => Some(Perk::NoFriendlyFireDamage),
            Self::LootTheKnockout => Some(Perk::AutoLoot),
            Self::SteadyBlade => Some(Perk::NoFriendlyFireKnockback),
            _ => None,
        }
    }

    /// Map a legacy perk to the node that now prices and gates it.
    pub const fn from_perk(perk: Perk) -> Self {
        match perk {
            Perk::KeepMovingInCombat => Self::FightThrough,
            Perk::AutoLoot => Self::LootTheKnockout,
            Perk::NoFriendlyFireDamage => Self::CarefulBlade,
            Perk::NoFriendlyFireKnockback => Self::SteadyBlade,
        }
    }

    /// The gameplay effect this node grants once owned.
    pub const fn effect(self) -> SkillEffect {
        match self {
            Self::FleetFooted => SkillEffect::SpeedFactor(FLEET_FOOTED_SPEED_FACTOR),
            Self::Hardy => SkillEffect::BonusLifePoints(HARDY_BONUS_LIFE_POINTS),
            Self::SilentStep => SkillEffect::WalkNoiseMultiplier(SILENT_STEP_NOISE_MULTIPLIER),
            Self::KeenEar => SkillEffect::ListenRadius(KEEN_EAR_LISTEN_RADIUS),
            Self::FightThrough => SkillEffect::FightThrough,
            Self::CarefulBlade => SkillEffect::CarefulBlade,
            Self::BurglarsKit => SkillEffect::LockpickAndClimb,
            Self::LootTheKnockout => SkillEffect::AutoLoot,
            Self::SteadyBlade => SkillEffect::SteadyBlade,
        }
    }

    /// A short player-facing label that names the tactical use, not a stat.
    pub const fn label(self) -> &'static str {
        match self {
            Self::FleetFooted => "Move faster on foot",
            Self::Hardy => "Survive more blows",
            Self::SilentStep => "Walk without being heard",
            Self::KeenEar => "Hear guards from further away",
            Self::FightThrough => "Fight through a swordfight",
            Self::CarefulBlade => "Strike allies without hurting them",
            Self::BurglarsKit => "Open locks and climb walls",
            Self::LootTheKnockout => "Loot a knocked-out guard",
            Self::SteadyBlade => "Never knock allies back",
        }
    }

    const fn bit(self) -> u16 {
        1u16 << self as u16
    }
}

// ─── Ownership set ───────────────────────────────────────────────

/// The nine owned flags in one `#[serde(default)]` field.  Nine bits fit in a
/// `u16` with room to append nodes; using a bitmask keeps the single schema
/// bump small and makes ownership deterministic in snapshots and state hashes.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SkillSet(u16);

impl SkillSet {
    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn has(self, node: SkillNode) -> bool {
        self.0 & node.bit() != 0
    }

    pub fn grant(&mut self, node: SkillNode) {
        self.0 |= node.bit();
    }

    /// Read the four pre-tree flags off a legacy `PcStatus`.  This is only a
    /// compatibility view: `PcDescription::has_skill` ORs it with the bitmask
    /// so a save written before the tree still reports its old purchases.
    pub fn from_status(status: &PcStatus) -> Self {
        let mut set = Self::empty();
        for node in SkillNode::ALL {
            if let Some(perk) = node.legacy_perk()
                && status.has_perk(perk)
            {
                set.grant(node);
            }
        }
        set
    }
}

// ─── Purchase rule ───────────────────────────────────────────────

/// Why a skill row cannot be bought, or that it can.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillPurchaseState {
    Affordable,
    Owned,
    PrerequisitesMissing,
    Unaffordable,
    WouldSpendLastClover,
}

impl SkillPurchaseState {
    pub const fn purchasable(self) -> bool {
        matches!(self, Self::Affordable)
    }
}

/// The one authoritative purchase rule for the tree.
///
/// `owns` answers ownership for any node of the same character, so prerequisites
/// are checked against the same store the engine uses to grant them.  The clover
/// floor is the VIP death save; the purse cap is enforced where clovers are
/// credited and cannot be exceeded by spending.
pub fn skill_purchase_state(
    node: SkillNode,
    clovers: i32,
    owns: impl Fn(SkillNode) -> bool,
) -> SkillPurchaseState {
    if owns(node) {
        return SkillPurchaseState::Owned;
    }
    if node.prerequisites().iter().any(|prereq| !owns(*prereq)) {
        return SkillPurchaseState::PrerequisitesMissing;
    }
    skill_purchase_state_ignoring_prerequisites(node, clovers, false)
}

/// The money gates shared by the tree and by the pre-tree flat-perk command.
///
/// The legacy `CampaignPurchasePerk` path predates tiers: Steady Blade could be
/// bought with no other perks.  Keeping it working is a save and wire-compat
/// requirement; only the price and the clover floor come from the tree there.
pub fn skill_purchase_state_ignoring_prerequisites(
    node: SkillNode,
    clovers: i32,
    owned: bool,
) -> SkillPurchaseState {
    if owned {
        return SkillPurchaseState::Owned;
    }
    let cost = node.cost();
    if clovers < cost {
        return SkillPurchaseState::Unaffordable;
    }
    if clovers - cost < crate::campaign::MINIMUM_CLOVERS_AFTER_PURCHASE {
        return SkillPurchaseState::WouldSpendLastClover;
    }
    SkillPurchaseState::Affordable
}

// ─── Panel row ───────────────────────────────────────────────────

/// One purchasable node on one campaign character, with the tree structure the
/// panel needs to draw tier headers and prerequisite arrows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillTreeRow {
    pub character_idx: u16,
    pub character_name: String,
    pub node: SkillNode,
    pub tier: u8,
    pub cost: i32,
    pub prerequisites: &'static [SkillNode],
    pub state: SkillPurchaseState,
}

/// Build the panel rows from campaign characters and the live clover purse.
pub fn skill_tree_rows(
    characters: &[crate::campaign::PcDescription],
    character_names: &[String],
    clovers: i32,
) -> Vec<SkillTreeRow> {
    assert_eq!(
        characters.len(),
        character_names.len(),
        "skill tree rows need one name per campaign character"
    );
    let mut rows = Vec::with_capacity(characters.len() * SkillNode::ALL.len());
    for (index, (character, name)) in characters.iter().zip(character_names).enumerate() {
        let character_idx = u16::try_from(index)
            .expect("campaign character index does not fit the purchase command's u16");
        for node in SkillNode::ALL {
            rows.push(SkillTreeRow {
                character_idx,
                character_name: name.clone(),
                node,
                tier: node.tier(),
                cost: node.cost(),
                prerequisites: node.prerequisites(),
                state: skill_purchase_state(node, clovers, |candidate| {
                    character.has_skill(candidate)
                }),
            });
        }
    }
    rows
}
