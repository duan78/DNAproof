//! HEDGES : code à hachis, décodé par une recherche best-first (Press et al., 2020).
//!
//! HEDGES (*Hash Encoded, Decoded by Greedy Exhaustive Search*) est un code « arbre » à
//! longueur de contrainte infinie, fait pour le canal ADN : il corrige les insertions et les
//! suppressions, pas seulement les substitutions, et il laisse la place aux contraintes
//! biochimiques sans perdre de rendement.
//!
//! **Encodage.** Chaque base porte zéro, un ou deux bits du message, selon un motif répété
//! ([`Rate`]). La base émise en position *i* est `A[(K + v) mod |A|]`, où `v` porte les bits du
//! message, `A` est l'ensemble des bases admises à cet endroit et `K` est un hachis de la
//! position, des derniers bits du message et du sel. Le message ne se lit donc pas dans l'ADN :
//! il se déduit du hachis, qui dépend de tout ce qui précède. Une erreur de lecture, ou de
//! position, dérègle toutes les prédictions qui suivent — et le décodeur s'en aperçoit.
//!
//! **Décodage.** Une recherche best-first sur l'arbre des hypothèses. Une hypothèse devine les
//! bits `v`, sa position dans le message et sa position dans la lecture ; chaque valeur de `v`
//! a trois sorties : la base lue correspond à la base émise, la lecture a une base de trop, ou
//! la base émise lui manque. Chaque pas coûte ce que coûte l'erreur qu'il implique, et rapporte
//! une petite récompense quand la base correspond. On développe toujours l'hypothèse la moins
//! chère : une lecture sans défaut ne coûte que ses développements, un par base du brin ; une
//! chaîne fausse paie vite et meurt.
//!
//! **Sel.** Les premiers [`SALT_BITS`] bits du message sont le sel : ils entrent dans le hachis
//! de toutes les bases qui suivent. Un index de brin mal décodé empoisonne le brin entier, qui
//! échoue : l'erreur d'index devient un effacement.
//!
//! **Contraintes.** Quand une base est interdite — qu'elle prolongerait une suite, ou ferait
//! sortir la part de GC du brin de l'intervalle 45 % – 55 % — l'index `(K + v) mod |A|` désigne
//! une base de `A`. Le rendement ne baisse pas : les bits que la base ne porte plus se
//! retrouvent plus loin, dans le hachis. La force de contrôle, elle, baisse : aussi petites
//! soient-elles, ces contraintes sont tenues pour rares (voir `State::allowed`).
//!
//! **Ce qui vient de l'article, ce qui est de ce module.** La construction, les motifs de bits,
//! les récompenses par rendement, la recherche best-first et son budget, la queue de zéros et le sel
//! sont ceux de l'article et de son implémentation de référence. Le hachis, la longueur de
//! l'historique (huit bits à ¼, douze à ⅓, seize à ½, quarante-huit à ¾, pour qu'une chaîne
//! fausse doive franchir seize contrôles avant de se resynchroniser) et les trois garde-fous du
//! décodage sont de ce module : une chaîne qui suppose trop d'erreurs est abandonnée
//! ([`Search`]), la queue de zéros est forcée et devient autant de contrôles purs, et un rival
//! d'un autre message, presque à égalité, fait échouer le décodage plutôt que de laisser passer
//! un message faux.

use crate::sequence::BASES;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Bits de sel : les premiers bits du message.
pub const SALT_BITS: usize = 24;
/// Bits de queue : deux octets de zéros, pour que les mauvaises chaînes soient mortes au bout du
/// message.
pub const MIN_RUNOUT_BITS: usize = 16;
/// Étiquette en clair d'un oligo à en-tête, en bases.
pub const HEADER_TAG_LEN: usize = 16;
/// Octets libres d'un oligo à en-tête : quatorze, pour que le manifeste tienne à tous les
/// rendements dans un brin de 300 bases.
pub const HEADER_LEN: usize = 14;
/// Sel des oligos à en-tête, connu d'avance : il n'y a pas d'index à protéger.
pub const HEADER_SALT: u32 = 0x48_45_44;

/// Bits de poids faible de la position qui entrent dans le hachis.
const POSITION_MASK: u32 = (1 << 12) - 1;
/// Marge de la contrainte de GC en tête de brin : l'écart entre le nombre de G et C et la moitié
/// des bases émises peut y atteindre ce nombre de bases (sur trente, c'est déjà 40 % à 60 %).
const GC_SLACK: i32 = 6;
/// Suite de bases identiques la plus longue admise.
const MAX_RUN: u32 = 3;
/// Coût d'une erreur, en millièmes de point.
const PENALTY: i32 = 1000;
/// Décalage qui rend les scores positifs dans la clé du tas.
const SCORE_BIAS: i64 = 1 << 20;
/// Son homologue dans la clé de la phase de contrôle, qui n'a que vingt bits pour le score.
const RIVAL_BIAS: i64 = 1 << 19;
/// Entrées au-delà desquelles le tas cède la place aux seaux.
const HEAP_CAPACITY: usize = 1 << 12;
/// Positions de queue, connues, qu'une lecture doit avoir lues pour être complétée : au-delà,
/// les bases manquantes ne coûtent rien ; en deçà, une lecture qui s'arrête est perdue.
///
/// La queue est ce qui arrête une chaîne fausse : sans elle, une lecture arrêtée au ras de la
/// charge utile laisse deviner le dernier bit — une suppression et une correspondance
/// chanceuse, une chance sur deux. Une position lue fait l'égalité, deux donnent la
/// préférence au bon message.
const MIN_READ_RUNOUT: usize = 2;

/// Développements au plus, par base du code, dans la phase de contrôle du gagnant.
const RIVAL_EXPANSIONS_PER_BASE: usize = 1;
/// Prédécesseur de la racine, dans l'arène.
const ROOT: u32 = u32::MAX;

/// Un rendement : un motif de bits par base, et les règles de la recherche.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rate {
    /// Motif, répété sur la longueur du brin.
    pub pattern: &'static [u8],
    /// Longueur d'un brin, en bases.
    pub chars: usize,
    /// Bits de message précédents qui entrent dans le hachis.
    pub history: u32,
    /// Récompense d'une base qui correspond : un score négatif, en millièmes de point. C'est la
    /// « gourmandise » de l'article, qui règle l'ardeur de la recherche à avancer.
    pub reward: i32,
    /// Taux d'erreurs, pour mille bases, au-delà duquel une chaîne est abandonnée.
    pub error_per_mille: u32,
}

impl Rate {
    /// Rendement ¼ : une base sur deux porte un bit. Brin de 300 bases, 150 bits.
    pub const QUARTER: Self = Self {
        pattern: &[1, 0],
        chars: 300,
        history: 8,
        reward: -265,
        error_per_mille: 150,
    };
    /// Rendement ⅓ : `[1, 1, 0]`. Brin de 300 bases, 200 bits.
    pub const THIRD: Self = Self {
        pattern: &[1, 1, 0],
        chars: 300,
        history: 12,
        reward: -229,
        error_per_mille: 150,
    };
    /// Rendement ½ : un bit de message par base. Brin de 200 bases, 200 bits.
    pub const HALF: Self = Self {
        pattern: &[1],
        chars: 200,
        history: 16,
        reward: -127,
        error_per_mille: 150,
    };
    /// Rendement ¾ : `[2, 1]`. Brin de 200 bases, 300 bits.
    pub const THREE_QUARTERS: Self = Self {
        pattern: &[2, 1],
        chars: 200,
        history: 48,
        reward: -35,
        error_per_mille: 60,
    };

    /// Tous les rendements, du plus sûr au plus dense.
    pub const ALL: [Self; 4] = [Self::QUARTER, Self::THIRD, Self::HALF, Self::THREE_QUARTERS];

    /// Bits de message que porte un brin.
    pub fn message_bits(&self) -> usize {
        (0..self.chars)
            .map(|position| usize::from(self.bits_at(position)))
            .sum()
    }

    /// Octets libres d'un brin : l'index et la charge utile.
    pub fn payload_bytes(&self) -> usize {
        (self.message_bits() - SALT_BITS - MIN_RUNOUT_BITS) / 8
    }

    /// Bits de queue : ce qui reste après le sel et la charge utile.
    pub fn runout_bits(&self) -> usize {
        self.message_bits() - SALT_BITS - 8 * self.payload_bytes()
    }

    /// Nom du rendement, pour la ligne de commande.
    pub fn name(&self) -> &'static str {
        match self.pattern {
            [1, 0] => "1/4",
            [1, 1, 0] => "1/3",
            [1] => "1/2",
            _ => "3/4",
        }
    }

    /// Code du rendement dans le manifeste.
    pub fn code(&self) -> u8 {
        match self.pattern {
            [1, 0] => 1,
            [1, 1, 0] => 2,
            [1] => 3,
            _ => 4,
        }
    }

    /// Le rendement d'un code de manifeste.
    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|rate| rate.code() == code)
    }

    /// Le rendement d'un nom de ligne de commande.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|rate| rate.name() == name.trim())
    }

    /// Nombre de bits de message de la base `position`.
    pub fn bits_at(&self, position: usize) -> u8 {
        self.pattern[position % self.pattern.len()]
    }
}

/// Ce qu'un oligo porte, et comment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub rate: Rate,
    /// Bases du code, étiquette en clair non comprise.
    pub chars: usize,
    /// Sel connu d'avance ; sinon les premiers bits du message servent de sel.
    pub salt: Option<u32>,
    /// Rang du premier bit libre dans le message.
    pub free_from: usize,
    /// Nombre de bits libres.
    pub free_len: usize,
}

impl Layout {
    /// Un brin de données : l'index sert de sel, la charge utile suit, la queue est connue.
    pub fn strand(rate: Rate) -> Self {
        Self {
            rate,
            chars: rate.chars,
            salt: None,
            free_from: 0,
            free_len: SALT_BITS + 8 * rate.payload_bytes(),
        }
    }

    /// Un oligo à en-tête : étiquette en clair, sel connu, puis [`HEADER_LEN`] octets libres.
    ///
    /// # Panics
    ///
    /// Si le rendement est trop dense pour que la queue de zéros tienne derrière l'en-tête.
    pub fn header(rate: Rate) -> Self {
        let chars = rate.chars - HEADER_TAG_LEN;
        let bits: usize = (0..chars)
            .map(|position| usize::from(rate.bits_at(position)))
            .sum();
        assert!(
            bits >= 8 * HEADER_LEN + MIN_RUNOUT_BITS,
            "rendement trop dense pour un oligo à en-tête"
        );
        Self {
            rate,
            chars,
            salt: Some(HEADER_SALT),
            free_from: 0,
            free_len: 8 * HEADER_LEN,
        }
    }

    /// Octets libres de ce code.
    pub fn free_bytes(&self) -> usize {
        self.free_len / 8
    }
}

#[inline]
fn mix(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

/// La clé `K` d'une position : un hachis de l'historique, de la position et du sel, ramené à
/// trente-deux bits.
#[inline]
fn hash_key(history: u64, position: usize, salt: u32) -> u32 {
    let input = (u128::from(history) << 44)
        | (u128::from((position as u32) & POSITION_MASK) << 32)
        | u128::from(salt);
    let folded = (input as u64) ^ ((input >> 64) as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93);
    (mix(folded) >> 32) as u32
}

/// Ce que l'encodeur et le décodeur savent d'un message en cours.
#[derive(Clone, Copy)]
struct State {
    /// Les derniers bits du message, le plus récent en poids faible.
    history: u64,
    salt: u32,
    /// Nombre de G et C émis.
    gc: u16,
    /// La dernière base émise, et la longueur de la suite de bases identiques qu'elle prolonge.
    last: u8,
    run: u8,
}

impl State {
    /// L'état d'avant la première base. Devant le brin, les bases sont réputées être `T`, `G`,
    /// `C`, `A`, répétés : la dernière est un `T`, qui ne prolonge aucune suite.
    fn start(salt: u32) -> Self {
        Self {
            history: 0,
            salt,
            gc: 0,
            last: 3,
            run: 1,
        }
    }

    /// La clé de hachis de la position `position`.
    fn key(&self, mask: u64, position: usize) -> u32 {
        hash_key(self.history & mask, position, self.salt)
    }

    /// Les bases admises à la suite, et leur nombre. `emitted` est le nombre de bases déjà émises.
    ///
    /// Deux règles se lisent sur le passé seul : pas de suite de plus de [`MAX_RUN`] bases
    /// identiques, et la part de GC du brin reste entre 45 % et 55 % — l'écart entre le nombre de
    /// G et C et la moitié des bases émises ne dépasse pas un dixième de leur nombre, ni
    /// [`GC_SLACK`] en tête de brin.
    ///
    /// Une base interdite ne coûte rien au rendement, mais la position ne contrôle plus rien
    /// quand il ne reste que deux bases admises : chacune des deux valeurs du bit de message y
    /// désigne une base, et une chaîne fausse trouve toujours la sienne. La règle de GC mord donc
    /// à la borne seulement ; une fenêtre glissante, qui la ferait mordre sans arrêt, a rendu le
    /// code aveugle sur les deux tiers du brin.
    fn allowed(&self, emitted: usize) -> ([u8; 4], u32) {
        // Écart actuel entre le nombre de G et C et la moitié des bases émises.
        let deviation = 2 * i32::from(self.gc) - emitted as i32;
        let slack = ((emitted as i32 + 1) / 10).max(GC_SLACK);
        // `true` : des G et des C seulement ; `false` : des A et des T seulement.
        let wanted = if deviation + 1 > slack {
            Some(false)
        } else if deviation - 1 < -slack {
            Some(true)
        } else {
            None
        };
        let (mut allowed, mut count) = ([0u8; 4], 0u32);
        for base in 0..4u8 {
            let gc = base == 1 || base == 2;
            let fits = wanted.is_none_or(|only_gc| gc == only_gc)
                && !(u32::from(self.run) >= MAX_RUN && base == self.last);
            if fits {
                allowed[count as usize] = base;
                count += 1;
            }
        }
        (allowed, count)
    }

    /// L'état après une base qui porte `value`, dont `salt_bits` bits comptent pour le sel.
    fn pushed(mut self, bits: u8, salt_bits: u8, value: u8, base: u8) -> Self {
        self.history = (self.history << bits) | u64::from(value);
        self.salt = (self.salt << salt_bits) | u32::from(value >> (bits - salt_bits));
        self.gc += u16::from(base == 1 || base == 2);
        // Une quatrième base identique est interdite : la suite ne dépasse jamais trois.
        self.run = if base == self.last { self.run + 1 } else { 1 };
        self.last = base;
        self
    }
}

/// Ce qu'une chaîne est autorisée à supposer, et ce qui fait échouer le décodage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Search {
    /// Hypothèses au plus ; au-delà, le décodage échoue.
    pub budget: usize,
    /// Erreurs qu'une chaîne peut déjà avoir supposées, en plus de sa part proportionnelle.
    pub base: u32,
    /// Écart de score, en millièmes de point, jusqu'où un rival d'un autre message fait échouer
    /// le décodage.
    pub margin: i32,
}

impl Default for Search {
    fn default() -> Self {
        Self {
            budget: 1 << 16,
            base: 6,
            margin: 1500,
        }
    }
}

impl Layout {
    /// Vrai si le bit de message de rang `rank` est libre, ni sel ni queue.
    fn is_free(&self, rank: usize) -> bool {
        rank >= self.free_from && rank < self.free_from + self.free_len
    }
}

/// Les tables d'un layout : ce que porte chaque position.
struct Frame {
    layout: Layout,
    /// Masque de l'historique, pour le hachis.
    history_mask: u64,
    /// Bits de message de chaque position.
    bits: Vec<u8>,
    /// Valeurs de `v` admises par chaque position : celles dont les bits connus sont nuls.
    masks: Vec<u8>,
    /// Bits de chaque position qui comptent pour le sel.
    salt_bits: Vec<u8>,
    /// Première position d'où une lecture épuisée est complétée sans coût.
    runout_from: usize,
}

impl Frame {
    fn new(layout: Layout) -> Self {
        assert!(layout.chars > 0, "un code d'au moins une base");
        let mut frame = Self {
            history_mask: (1u64 << layout.rate.history) - 1,
            bits: Vec::with_capacity(layout.chars),
            masks: Vec::with_capacity(layout.chars),
            salt_bits: Vec::with_capacity(layout.chars),
            runout_from: 0,
            layout,
        };
        let mut rank = 0;
        for position in 0..frame.layout.chars {
            let count = frame.layout.rate.bits_at(position);
            let mut mask = 0;
            for value in 0..(1u16 << count) {
                let known_zero = (0..count).all(|bit| {
                    layout.is_free(rank + usize::from(bit))
                        || value >> usize::from(count - 1 - bit) & 1 == 0
                });
                if known_zero {
                    mask |= 1 << value;
                }
            }
            frame.bits.push(count);
            frame.masks.push(mask as u8);
            let salt = if frame.layout.salt.is_some() {
                0
            } else {
                (0..count)
                    .filter(|bit| rank + usize::from(*bit) < SALT_BITS)
                    .count() as u8
            };
            frame.salt_bits.push(salt);
            rank += usize::from(count);
        }
        assert!(
            rank >= frame.layout.free_from + frame.layout.free_len,
            "le code est trop court pour ses bits libres"
        );
        // La queue : les dernières positions qui ne portent plus aucun bit libre.
        let tail = frame
            .masks
            .iter()
            .rev()
            .take_while(|&&mask| mask == 1)
            .count();
        frame.runout_from = (frame.layout.chars - tail + MIN_READ_RUNOUT).min(frame.layout.chars);
        frame
    }
}

/// La clé du tas pendant la phase de contrôle : le score final escompté, puis la distance au bout
/// — les plus avancées d'abord, qui sont les moins chères à vérifier et les rivales les plus
/// plausibles —, puis le rang. Le rang reste en clair dans les 32 bits de poids faible.
#[inline]
fn rival_key(hopeful: i32, remaining: usize, id: u32) -> u64 {
    let score = ((i64::from(hopeful) + RIVAL_BIAS) as u64) & ((1 << 20) - 1);
    score << 44 | (u64::from(remaining as u16) & 0xFFF) << 32 | u64::from(id)
}

/// Une hypothèse de la recherche : tout ce qu'il faut pour la développer.
#[derive(Clone, Copy)]
struct Node {
    /// Prédécesseur, dans l'arène ; [`ROOT`] pour la racine.
    previous: u32,
    /// Somme des coûts et des récompenses, en millièmes de point.
    score: i32,
    state: State,
    /// Prochaine base de la lecture à comparer.
    at: u16,
    /// Position de la base émise, dans le code ; `u16::MAX` pour la racine.
    position: u16,
    /// Erreurs supposées.
    errors: u16,
    /// Bits de message portés par cette base.
    value: u8,
}

/// La clé du tas : le score, puis le rang de création — les aînées d'abord à égalité.
#[inline]
fn heap_key(score: i32, id: u32) -> u64 {
    ((i64::from(score) + SCORE_BIAS) as u64) << 32 | u64::from(id)
}

#[inline]
fn key_score(key: u64) -> i32 {
    ((key >> 32) as i64 - SCORE_BIAS) as i32
}

/// Un rang qui n'a pas de suivant dans son seau.
const NO_NEXT: u32 = u32::MAX;

/// Des hypothèses en attente dans des seaux : un par score et, dans un seau, les aînées d'abord.
/// C'est l'ordre d'un tas trié sur le score puis le rang de création, en travail constant par
/// hypothèse.
struct Waiting {
    /// Score du seau de rang zéro : celui d'une lecture entièrement correspondante.
    origin: i32,
    /// Têtes et queues des seaux. Les huit bits de poids fort portent l'époque de la lecture, les
    /// vingt-quatre autres le rang de l'hypothèse : un seau d'une autre époque est vide, et il
    /// n'y a rien à effacer d'une lecture à l'autre.
    heads: Vec<u32>,
    tails: Vec<u32>,
    /// L'hypothèse qui suit dans son seau, par rang.
    following: Vec<u32>,
    /// Un bit par seau : non vide. Seuls les seaux touchés par la lecture sont remis à zéro.
    occupied: Vec<u64>,
    /// Premier et dernier seau devenus non vides pendant la lecture.
    span: (usize, usize),
    epoch: u32,
    /// Seau de la moins chère hypothèse en attente, ou celui de la dernière sortie : la suivante
    /// n'est jamais en dessous.
    lowest: usize,
    len: usize,
}

impl Waiting {
    fn new(origin: i32, width: usize, budget: usize) -> Self {
        Self {
            origin,
            heads: vec![0; width],
            tails: vec![0; width],
            following: vec![NO_NEXT; budget.max(1)],
            occupied: vec![0; width.div_ceil(64)],
            span: (usize::MAX, 0),
            epoch: 1,
            lowest: 0,
            len: 0,
        }
    }

    /// Ajuste la mémoire à de nouvelles règles, entre deux lectures.
    fn fit(&mut self, origin: i32, width: usize, budget: usize) {
        if origin != self.origin || self.heads.len() < width {
            self.heads.resize(width, 0);
            self.tails.resize(width, 0);
            self.occupied.resize(width.div_ceil(64), 0);
            self.origin = origin;
            self.span = (usize::MAX, 0);
            self.lowest = 0;
            self.len = 0;
            // Les seaux sont vides, à l'époque zéro : la prochaine lecture en change.
            self.epoch = 0;
        }
        if self.following.len() < budget.max(1) {
            self.following.resize(budget.max(1), NO_NEXT);
        }
    }

    /// Oublie la lecture précédente.
    fn reset(&mut self) {
        let (low, high) = self.span;
        if low <= high {
            self.occupied[low >> 6..=high >> 6].fill(0);
        }
        self.span = (usize::MAX, 0);
        self.lowest = 0;
        self.len = 0;
        self.epoch += 1;
        if self.epoch > 0xFF {
            self.epoch = 1;
            self.heads.fill(0);
            self.tails.fill(0);
        }
    }

    #[inline(always)]
    fn push(&mut self, score: i32, id: u32) {
        let bucket = (score - self.origin) as usize;
        let head = &mut self.heads[bucket];
        if *head >> 24 != self.epoch {
            *head = self.epoch << 24 | id;
            self.occupied[bucket >> 6] |= 1 << (bucket & 63);
            if bucket < self.span.0 {
                self.span.0 = bucket;
            }
            if bucket > self.span.1 {
                self.span.1 = bucket;
            }
            if bucket < self.lowest {
                self.lowest = bucket;
            }
        } else {
            self.following[(self.tails[bucket] & 0x00FF_FFFF) as usize] = id;
        }
        self.tails[bucket] = self.epoch << 24 | id;
        self.following[id as usize] = NO_NEXT;
        self.len += 1;
    }

    /// La moins chère hypothèse, et son score.
    #[inline]
    fn pop(&mut self) -> Option<(i32, u32)> {
        if self.len == 0 {
            return None;
        }
        // Tout ce qui reste est au-dessus du dernier sorti.
        let mut bucket = self.lowest;
        let mut bits = self.occupied[bucket >> 6] >> (bucket & 63);
        while bits == 0 {
            bucket = (bucket & !63) + 64;
            bits = *self
                .occupied
                .get(bucket >> 6)
                .expect("une hypothèse en attente a un seau");
        }
        bucket += bits.trailing_zeros() as usize;
        let id = self.heads[bucket] & 0x00FF_FFFF;
        let next = self.following[id as usize];
        if next == NO_NEXT {
            self.heads[bucket] = 0;
            self.occupied[bucket >> 6] &= !(1 << (bucket & 63));
        } else {
            self.heads[bucket] = self.epoch << 24 | next;
        }
        self.lowest = bucket;
        self.len -= 1;
        Some((self.origin + bucket as i32, id))
    }

    /// Vide la file : les hypothèses du seau le plus bas au plus haut.
    fn drain(&mut self) -> Draining<'_> {
        let (low, high) = self.span;
        Draining {
            waiting: self,
            word: if low <= high { low >> 6 } else { 0 },
            last_word: high >> 6,
            bits: 0,
            id: NO_NEXT,
        }
    }
}

/// Le parcours d'une file qu'on vide.
struct Draining<'a> {
    waiting: &'a mut Waiting,
    word: usize,
    last_word: usize,
    bits: u64,
    id: u32,
}

impl Iterator for Draining<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        loop {
            if self.id != NO_NEXT {
                let id = self.id;
                self.id = self.waiting.following[id as usize];
                return Some(id);
            }
            while self.bits == 0 {
                if self.word > self.last_word {
                    self.waiting.len = 0;
                    return None;
                }
                self.bits = self.waiting.occupied[self.word];
                self.word += 1;
            }
            let bucket = (self.word - 1) * 64 + self.bits.trailing_zeros() as usize;
            self.bits &= self.bits - 1;
            self.id = self.waiting.heads[bucket] & 0x00FF_FFFF;
        }
    }
}

/// Les hypothèses en attente, la moins chère d'abord : le score, puis le rang de création.
///
/// Tant que la recherche va bon train, un tas binaire suffit : ses quelques milliers d'entrées
/// tiennent dans le cache. Quand elle s'enlise, il cède la place à des seaux, où chaque opération
/// est en travail constant : une recherche sans issue en compte des dizaines de milliers. Les
/// seaux ne sont alloués qu'au moment du débordement — un décodeur qui ne lit que des lectures
/// saines reste bon marché à créer.
struct Pending {
    heap: BinaryHeap<Reverse<u64>>,
    /// Les seaux, pris la première fois que le tas a débordé [`HEAP_CAPACITY`].
    buckets: Option<Waiting>,
    bucketed: bool,
    origin: i32,
    width: usize,
}

impl Pending {
    fn new(origin: i32, width: usize) -> Self {
        Self {
            heap: BinaryHeap::new(),
            buckets: None,
            bucketed: false,
            origin,
            width,
        }
    }

    /// Oublie la lecture précédente.
    fn reset(&mut self) {
        self.heap.clear();
        self.bucketed = false;
        if let Some(buckets) = &mut self.buckets {
            buckets.reset();
        }
    }

    /// Agrandit les seaux pour un budget plus large, entre deux lectures.
    fn fit(&mut self, budget: usize) {
        if let Some(buckets) = &mut self.buckets {
            buckets.fit(self.origin, self.width, budget);
        }
    }

    #[inline(always)]
    fn push(&mut self, score: i32, id: u32, budget: usize) {
        if self.bucketed {
            self.buckets
                .as_mut()
                .expect("les seaux sont pris avant que la file y passe")
                .push(score, id);
        } else {
            self.heap.push(Reverse(heap_key(score, id)));
            if self.heap.len() >= HEAP_CAPACITY {
                self.spill(budget);
            }
        }
    }

    /// Le tas a débordé : ses hypothèses passent dans les seaux, qui ne coûtent rien tant que
    /// la recherche ne s'enlise pas.
    ///
    /// Dans un seau, l'ordre est celui d'arrivée : il faut lui donner celui du tas, le rang à
    /// score égal — sinon, à égalité, la recherche ne développerait plus les aînées d'abord.
    #[cold]
    fn spill(&mut self, budget: usize) {
        let mut entries = std::mem::take(&mut self.heap).into_vec();
        entries.sort_unstable_by_key(|entry| entry.0);
        let buckets = self
            .buckets
            .get_or_insert_with(|| Waiting::new(self.origin, self.width, budget));
        for Reverse(key) in entries {
            buckets.push(key_score(key), key as u32);
        }
        self.bucketed = true;
    }

    /// La moins chère hypothèse, et son score.
    #[inline(always)]
    fn pop(&mut self) -> Option<(i32, u32)> {
        if self.bucketed {
            self.buckets
                .as_mut()
                .expect("les seaux sont pris avant que la file y passe")
                .pop()
        } else {
            self.heap
                .pop()
                .map(|Reverse(key)| (key_score(key), key as u32))
        }
    }

    /// Vide la file, dans n'importe quel ordre.
    fn drain(&mut self, mut give: impl FnMut(u32)) {
        for Reverse(key) in std::mem::take(&mut self.heap) {
            give(key as u32);
        }
        if let Some(buckets) = &mut self.buckets {
            for id in buckets.drain() {
                give(id);
            }
        }
    }
}

/// La fin d'un décodage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Un message, sans rival.
    Solved,
    /// Aucune chaîne n'est arrivée au bout : les erreurs sont trop nombreuses, ou la lecture
    /// n'est pas celle de ce code.
    Lost,
    /// Le budget d'hypothèses est épuisé avant qu'une chaîne n'arrive au bout.
    Exhausted,
    /// Un autre message explique la lecture presque aussi bien : on préfère ne rien dire.
    Rival,
    /// La lecture contient autre chose que A, C, G et T, ou plus de 65 535 bases.
    Invalid,
}

/// Le résultat d'un décodage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    /// Comment le décodage s'est terminé.
    pub outcome: Outcome,
    /// Les octets libres du message, si le décodage a réussi.
    pub message: Option<Vec<u8>>,
    /// Score de la chaîne retenue, en millièmes de point.
    pub score: i32,
    /// Erreurs que la chaîne retenue suppose.
    pub errors: u16,
    /// Hypothèses créées.
    pub hypotheses: u32,
}

impl Decoded {
    fn failed(outcome: Outcome, hypotheses: u32) -> Self {
        Self {
            outcome,
            message: None,
            score: 0,
            errors: 0,
            hypotheses,
        }
    }

    /// Vrai si un message a été rendu.
    pub fn is_solved(&self) -> bool {
        self.outcome == Outcome::Solved
    }
}

/// Les scores qu'une hypothèse peut prendre : depuis la lecture entièrement correspondante
/// jusqu'au plafond d'erreurs en fin de brin. Une erreur ne coûte jamais plus d'un point, une base
/// qui correspond rapporte toujours sa récompense.
fn score_range(frame: &Frame, search: &Search) -> (i32, usize) {
    let rate = frame.layout.rate;
    let chars = frame.layout.chars;
    let top = 1000 * (search.base + rate.error_per_mille * (chars - 1) as u32 / 1000);
    let origin = rate.reward * chars as i32;
    (origin, (i64::from(top) - i64::from(origin) + 1) as usize)
}

/// Décodeur : les tables d'un layout et la mémoire de sa recherche, réutilisées d'une lecture à
/// l'autre.
pub struct Decoder {
    frame: Frame,
    search: Search,
    /// Les bases de la lecture, en valeurs de 0 à 3.
    read: Vec<u8>,
    /// Les bits de message de chaque position, en remontant la chaîne retenue.
    values: Vec<u8>,
    /// Les hypothèses créées.
    arena: Vec<Node>,
    pending: Pending,
    /// Le tas de la phase de contrôle du gagnant.
    rival: BinaryHeap<Reverse<u64>>,
    /// Les rangs des hypothèses en attente, le temps de les ranger pour la phase de contrôle.
    order: Vec<u32>,
}

impl Decoder {
    /// Un décodeur aux règles par défaut.
    pub fn new(layout: Layout) -> Self {
        Self::with_search(layout, Search::default())
    }

    /// Un décodeur aux règles données.
    pub fn with_search(layout: Layout, search: Search) -> Self {
        let frame = Frame::new(layout);
        let (origin, width) = score_range(&frame, &search);
        Self {
            pending: Pending::new(origin, width),
            frame,
            search,
            read: Vec::new(),
            values: vec![0; layout.chars],
            arena: Vec::new(),
            rival: BinaryHeap::new(),
            order: Vec::new(),
        }
    }

    /// Les règles de la recherche.
    pub fn search(&self) -> Search {
        self.search
    }

    /// Change les règles de la recherche — le budget, la ligne d'abandon, la marge du rival —,
    /// entre deux lectures. La mémoire s'agrandit s'il le faut.
    pub fn set_search(&mut self, search: Search) {
        let (origin, width) = score_range(&self.frame, &search);
        if (origin, width) != (self.pending.origin, self.pending.width) {
            self.pending = Pending::new(origin, width);
        } else {
            self.pending.fit(search.budget);
        }
        self.search = search;
    }

    /// Décode une lecture.
    ///
    /// La lecture peut compter plus ou moins de bases que l'oligo : c'est fait pour. Le décodage
    /// réussit si une chaîne arrive au bout du message en supposant assez peu d'erreurs, et si
    /// aucun message rival ne finit presque aussi bien.
    pub fn decode(&mut self, read: &[u8]) -> Decoded {
        if read.len() >= usize::from(u16::MAX) {
            return Decoded::failed(Outcome::Invalid, 0);
        }
        let Self {
            frame,
            search,
            read: bases,
            values,
            arena,
            pending,
            rival,
            order,
        } = self;
        bases.clear();
        bases.extend(read.iter().map(|&base| match base {
            b'A' => 0,
            b'C' => 1,
            b'G' => 2,
            b'T' => 3,
            _ => 255,
        }));
        if bases.contains(&255) {
            return Decoded::failed(Outcome::Invalid, 0);
        }
        let read = bases.as_slice();
        let length = read.len();
        let chars = frame.layout.chars;
        let last = chars - 1;
        let reward = frame.layout.rate.reward;
        let per_mille = frame.layout.rate.error_per_mille;
        // Erreurs qu'une chaîne peut avoir supposées en arrivant à cette position.
        let ceiling = |position: usize| search.base + per_mille * position as u32 / 1000;
        // Chaque position consume zéro, une ou deux bases de la lecture : pour tout expliquer,
        // il faut au moins autant d'insertions et de suppressions que la lecture a de bases en
        // trop ou de moins. Au-delà de la ligne, rien ne peut aboutir : inutile de chercher.
        let slack = ceiling(chars) as usize;
        if length + slack < chars || length > chars + slack {
            return Decoded::failed(Outcome::Lost, 1);
        }
        // Une lecture doit avoir lu deux positions de la queue : moins, le dernier bit libre
        // pourrait se deviner.
        if length < frame.runout_from {
            return Decoded::failed(Outcome::Lost, 1);
        }

        arena.clear();
        rival.clear();
        pending.reset();
        arena.push(Node {
            previous: ROOT,
            score: 0,
            state: State::start(frame.layout.salt.unwrap_or(0)),
            at: 0,
            position: u16::MAX,
            errors: 0,
            value: 0,
        });
        pending.push(0, 0, search.budget);
        // La première chaîne arrivée au bout, et son score.
        let mut winner: Option<(u32, i32)> = None;
        let mut exhausted = false;
        // Développements faits pendant la phase de contrôle.
        let mut expansions = 0usize;
        // Vrai pendant le contrôle du gagnant : les hypothèses en attente sont alors au tas, par
        // score final escompté.
        let mut rivalry = false;
        loop {
            let id = if rivalry {
                let Some(Reverse(key)) = rival.pop() else {
                    break;
                };
                key as u32
            } else {
                let Some((_, id)) = pending.pop() else {
                    break;
                };
                id
            };
            let hypothesis = arena[id as usize];
            let remaining = usize::from((last as u16).wrapping_sub(hypothesis.position));
            if usize::from(hypothesis.position) == last {
                match winner {
                    None => {
                        winner = Some((id, hypothesis.score));
                        // Un gagnant sans erreur n'a pas de rival : il faudrait qu'un autre
                        // message ait franchi toutes les vérifications par chance.
                        if hypothesis.errors == 0 {
                            break;
                        }
                        // Désormais le tas est ordonné par le score final le plus bas que chaque
                        // hypothèse peut encore atteindre : le sien si tout le reste de la
                        // lecture correspondait. Un rival en attente n'est plus pénalisé par les
                        // récompenses qui lui restent à prendre.
                        order.clear();
                        pending.drain(|id| order.push(id));
                        rival.extend(order.drain(..).map(|id| {
                            let node = arena[id as usize];
                            let remaining = usize::from((last as u16).wrapping_sub(node.position));
                            Reverse(rival_key(
                                node.score + reward * remaining as i32,
                                remaining,
                                id,
                            ))
                        }));
                        rivalry = true;
                    }
                    Some((first, _)) if !same_message(arena, first, id) => {
                        return Decoded::failed(Outcome::Rival, arena.len() as u32);
                    }
                    Some(_) => {}
                }
                continue;
            }
            if let Some((_, best)) = winner {
                let hopeful = hypothesis.score + reward * remaining as i32;
                // Plus personne ne peut finir à moins d'une marge du gagnant ; ou la vérification
                // en a assez coûté, et le gagnant garde le bénéfice du doute.
                if hopeful > best + search.margin || expansions == RIVAL_EXPANSIONS_PER_BASE * chars
                {
                    break;
                }
                expansions += 1;
            }

            let position = usize::from(hypothesis.position.wrapping_add(1));
            let bits = frame.bits[position];
            let (allowed, count) = hypothesis.state.allowed(position);
            let key = hypothesis.state.key(frame.history_mask, position);
            let at = usize::from(hypothesis.at);
            let final_row = position == last;
            let free_tail = position >= frame.runout_from;
            for value in 0..4u8 {
                if frame.masks[position] & (1 << value) == 0 {
                    continue;
                }
                let base = allowed[(key.wrapping_add(u32::from(value)) % count) as usize];
                let state = hypothesis
                    .state
                    .pushed(bits, frame.salt_bits[position], value, base);
                for skew in [-1i8, 0, 1] {
                    // Le coût de ce pas, les erreurs qu'il suppose, la prochaine base à comparer.
                    let step = match skew {
                        // La base émise manque dans la lecture.
                        -1 if at < length => Some((PENALTY, 1, at)),
                        // La lecture est épuisée, et elle est entrée dans la queue : la base ne
                        // manque pas, elle est connue, et la lecture s'est simplement arrêtée là.
                        // Rien à payer. Plus tôt, une lecture épuisée n'a plus de suite.
                        -1 if at == length && free_tail => Some((0, 0, at)),
                        0 if at < length => {
                            let (cost, errors) = comparison(read, at, base, reward);
                            Some((cost, errors, at + 1))
                        }
                        // La lecture a une base de trop : on la saute.
                        1 if at + 1 < length => {
                            let (cost, errors) = comparison(read, at + 1, base, reward);
                            Some((PENALTY + cost, errors + 1, at + 2))
                        }
                        _ => None,
                    };
                    let Some((cost, errors, next)) = step else {
                        continue;
                    };
                    let mut errors = hypothesis.errors + errors;
                    let mut score = hypothesis.score + cost;
                    if final_row {
                        // Ce qui reste en fin de lecture est compté comme inséré.
                        let trailing = (length - next) as u16;
                        score += PENALTY * i32::from(trailing);
                        errors += trailing;
                    }
                    if u32::from(errors) > ceiling(position) {
                        continue;
                    }
                    if arena.len() >= search.budget {
                        // Plus de place. Si personne n'est encore arrivé au bout, la bonne
                        // chaîne a pu être coupée là : une chaîne qui achèvera plus tard n'en
                        // vaut rien.
                        exhausted |= winner.is_none();
                        continue;
                    }
                    let child = arena.len() as u32;
                    arena.push(Node {
                        previous: id,
                        score,
                        state,
                        at: next as u16,
                        position: position as u16,
                        errors,
                        value,
                    });
                    if rivalry {
                        rival.push(Reverse(rival_key(
                            score + reward * (last - position) as i32,
                            last - position,
                            child,
                        )));
                    } else {
                        pending.push(score, child, search.budget);
                    }
                }
            }
        }

        if exhausted {
            return Decoded::failed(Outcome::Exhausted, arena.len() as u32);
        }
        let Some((id, score)) = winner else {
            return Decoded::failed(Outcome::Lost, arena.len() as u32);
        };
        let winner = arena[id as usize];
        Decoded {
            outcome: Outcome::Solved,
            message: Some(message_of(frame, arena, id, values)),
            score,
            errors: winner.errors,
            hypotheses: arena.len() as u32,
        }
    }
}

/// Le coût d'une base lue comparée à la base émise : une récompense, ou une substitution.
#[inline]
fn comparison(read: &[u8], at: usize, base: u8, reward: i32) -> (i32, u16) {
    if read[at] == base {
        (reward, 0)
    } else {
        (PENALTY, 1)
    }
}

/// Si deux chaînes parvenues au bout portent le même message.
///
/// Elles ont une base par position, de la première à la dernière : leurs bits se comparent
/// en remontant l'une et l'autre. C'est rare — il faut qu'un rival finisse —, et ça dispense
/// de tenir l'empreinte de chaque hypothèse.
fn same_message(arena: &[Node], mut first: u32, mut other: u32) -> bool {
    while first != 0 && other != 0 {
        let (one, two) = (arena[first as usize], arena[other as usize]);
        if one.position != two.position || one.value != two.value {
            return false;
        }
        (first, other) = (one.previous, two.previous);
    }
    first == other
}

/// Remonte la chaîne retenue et en tire les octets libres.
fn message_of(frame: &Frame, arena: &[Node], mut id: u32, values: &mut [u8]) -> Vec<u8> {
    values.fill(0);
    // La racine, de rang nul, n'a pas de base.
    while id != 0 {
        let node = &arena[id as usize];
        values[usize::from(node.position)] = node.value;
        id = node.previous;
    }
    let layout = &frame.layout;
    let mut message = vec![0u8; layout.free_bytes()];
    let mut rank = 0;
    for (position, &bits) in frame.bits.iter().enumerate() {
        for bit in (0..bits).rev() {
            let free = rank >= layout.free_from && rank < layout.free_from + layout.free_len;
            if free && values[position] >> bit & 1 == 1 {
                let index = rank - layout.free_from;
                message[index / 8] |= 1 << (7 - index % 8);
            }
            rank += 1;
        }
    }
    message
}

/// Encodeur : les tables d'un layout et le brin en cours, réutilisés d'un message à l'autre.
pub struct Encoder {
    frame: Frame,
    dna: Vec<u8>,
}

impl Encoder {
    /// Un encodeur pour ce layout.
    pub fn new(layout: Layout) -> Self {
        Self {
            frame: Frame::new(layout),
            dna: Vec::with_capacity(layout.chars),
        }
    }

    /// Le brin qui porte les octets libres d'un message.
    ///
    /// Les autres bits du message — le sel quand il n'est pas connu, la queue — sont nuls. Le
    /// brin rendu se lit jusqu'au prochain encodage.
    ///
    /// # Panics
    ///
    /// Si `free` n'a pas la longueur des octets libres du layout.
    pub fn encode(&mut self, free: &[u8]) -> &[u8] {
        let frame = &self.frame;
        let layout = &frame.layout;
        assert_eq!(
            free.len(),
            layout.free_bytes(),
            "longueur de message inattendue"
        );
        let mut state = State::start(layout.salt.unwrap_or(0));
        self.dna.clear();
        let mut rank = 0;
        for position in 0..layout.chars {
            let bits = frame.bits[position];
            let mut value = 0;
            for _ in 0..bits {
                let next = if layout.is_free(rank) {
                    let index = rank - layout.free_from;
                    free[index / 8] >> (7 - index % 8) & 1
                } else {
                    0
                };
                value = value << 1 | next;
                rank += 1;
            }
            let (allowed, count) = state.allowed(position);
            let key = state.key(frame.history_mask, position);
            let base = allowed[key.wrapping_add(u32::from(value)) as usize % count as usize];
            self.dna.push(BASES[usize::from(base)]);
            state = state.pushed(bits, frame.salt_bits[position], value, base);
        }
        &self.dna
    }
}

/// Encode les octets libres d'un layout en ADN, sans réutiliser la mémoire d'un encodeur
/// ([`Encoder::encode`]).
pub fn encode(layout: &Layout, free: &[u8]) -> Vec<u8> {
    Encoder::new(*layout).encode(free).to_vec()
}

/// Décode une lecture avec les règles données, sans réutiliser la mémoire d'un décodeur.
pub fn decode(layout: &Layout, read: &[u8], search: Search) -> Decoded {
    Decoder::with_search(*layout, search).decode(read)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::rng::SeededRng;
    use crate::constraints::DnaConstraints;

    fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
        let mut rng = SeededRng::new(seed, 0x61);
        let mut out = vec![0u8; len];
        rng.fill_bytes(&mut out);
        out
    }

    /// Une lecture abîmée : chaque base est remplacée, ou précède une base insérée, ou
    /// disparaît, avec les taux (substitution, insertion, suppression).
    fn damage(dna: &[u8], rates: (f64, f64, f64), rng: &mut SeededRng) -> Vec<u8> {
        let mut read = Vec::with_capacity(dna.len() + 8);
        for &base in dna {
            let roll = rng.next_f64();
            let rank = BASES.iter().position(|&known| known == base).unwrap_or(0);
            if roll < rates.0 {
                read.push(BASES[(rank + 1 + rng.below(3) as usize) % 4]);
            } else if roll < rates.0 + rates.1 {
                read.push(BASES[rng.below(4) as usize]);
                read.push(base);
            } else if roll < rates.0 + rates.1 + rates.2 {
                // La base disparaît.
            } else {
                read.push(base);
            }
        }
        read
    }

    /// Part des lectures restituées, et nombre de messages faux.
    fn restored(rate: Rate, rates: (f64, f64, f64), trials: usize) -> (usize, usize) {
        let layout = Layout::strand(rate);
        let mut decoder = Decoder::new(layout);
        let mut rng = SeededRng::new(77, 0x62);
        let (mut solved, mut wrong) = (0, 0);
        for trial in 0..trials {
            let free = pseudo_random(
                layout.free_bytes(),
                1000 + trial as u64 + u64::from(rate.code()) * 10_000,
            );
            let dna = encode(&layout, &free);
            let read = damage(&dna, rates, &mut rng);
            let decoded = decoder.decode(&read);
            if let Some(found) = &decoded.message {
                solved += 1;
                wrong += usize::from(found != &free);
            }
        }
        (solved, wrong)
    }

    #[test]
    fn rates_follow_the_article() {
        assert_eq!(Rate::QUARTER.pattern, &[1, 0]);
        assert_eq!(Rate::THIRD.pattern, &[1, 1, 0]);
        assert_eq!(Rate::HALF.pattern, &[1]);
        assert_eq!(Rate::THREE_QUARTERS.pattern, &[2, 1]);
        for rate in Rate::ALL {
            // Le rendement, tel que l'article le nomme, vaut la moitié de la moyenne du motif :
            // un bit de message par base de code à ½, la moitié à ¼.
            let mean = rate
                .pattern
                .iter()
                .map(|&bits| f64::from(bits))
                .sum::<f64>()
                / rate.pattern.len() as f64;
            let (numerator, denominator) = rate.name().split_once('/').expect("un nom en fraction");
            let named = numerator.parse::<f64>().unwrap() / denominator.parse::<f64>().unwrap();
            assert!((named - mean / 2.0).abs() < 1e-12, "{}", rate.name());
            assert_eq!(
                rate.message_bits() * rate.pattern.len(),
                usize::from(rate.pattern.iter().sum::<u8>()) * rate.chars
            );
            assert_eq!(
                rate.runout_bits(),
                rate.message_bits() - SALT_BITS - 8 * rate.payload_bytes()
            );
            assert!(rate.runout_bits() >= MIN_RUNOUT_BITS, "{}", rate.name());
            assert!(rate.runout_bits() < MIN_RUNOUT_BITS + 8, "{}", rate.name());
            assert_eq!(Rate::from_code(rate.code()), Some(rate));
            assert_eq!(Rate::from_name(rate.name()), Some(rate));
            assert_eq!(Rate::from_name(&format!(" {} ", rate.name())), Some(rate));
            // Seize contrôles au moins séparent une chaîne fausse de sa resynchronisation.
            let (mut bits, mut checks, mut position) = (0u32, 0, 0);
            while bits < rate.history {
                let count = u32::from(rate.bits_at(position));
                bits += count;
                checks += 2 - count;
                position += 1;
            }
            assert!(checks >= 16, "{}", rate.name());
        }
        assert_eq!(Rate::QUARTER.message_bits(), 150);
        assert_eq!(Rate::THIRD.message_bits(), 200);
        assert_eq!(Rate::HALF.message_bits(), 200);
        assert_eq!(Rate::THREE_QUARTERS.message_bits(), 300);
        assert_eq!(Rate::QUARTER.payload_bytes(), 13);
        assert_eq!(Rate::THIRD.payload_bytes(), 20);
        assert_eq!(Rate::HALF.payload_bytes(), 20);
        assert_eq!(Rate::THREE_QUARTERS.payload_bytes(), 32);
        assert!(Rate::from_code(5).is_none());
        assert!(Rate::from_name("1/6").is_none());
    }

    #[test]
    fn a_clean_read_gives_back_its_message_and_respects_the_constraints() {
        let constraints = DnaConstraints::default();
        for rate in Rate::ALL {
            for layout in [Layout::strand(rate), Layout::header(rate)] {
                let mut decoder = Decoder::new(layout);
                for seed in 0..30u64 {
                    let free = pseudo_random(layout.free_bytes(), seed);
                    let dna = encode(&layout, &free);
                    assert_eq!(dna.len(), layout.chars);
                    constraints.check(&dna).unwrap();
                    let decoded = decoder.decode(&dna);
                    assert_eq!(decoded.outcome, Outcome::Solved, "{}", rate.name());
                    assert_eq!(decoded.message.as_deref(), Some(free.as_slice()));
                    assert_eq!(decoded.errors, 0);
                    assert_eq!(decoded.score, rate.reward * layout.chars as i32);
                    // Une lecture sans défaut ne développe que sa chaîne : ses enfants, et ceux
                    // de ses jumeaux aux rares positions contraintes (12 enfants par base à ¾).
                    let per_base = if rate == Rate::THREE_QUARTERS { 64 } else { 10 };
                    assert!(
                        decoded.hypotheses as usize <= per_base * layout.chars,
                        "{} : {} hypothèses",
                        rate.name(),
                        decoded.hypotheses
                    );
                }
            }
        }
    }

    #[test]
    fn substitutions_insertions_and_deletions_are_corrected() {
        // Les parts minimales suivent les mesures (300 lectures par cas) : à rendement égal, plus
        // les erreurs sont nombreuses, plus la recherche bute sur son budget.
        //
        // `wrong` compte les messages faux rendus. À 6 % d'erreurs ou moins il n'y en a aucun.
        // À ¼, dès 10 %, il en passe trois à quatre pour mille : le vrai chemin n'est plus
        // toujours le moins cher, un autre message absorbe une grappe d'erreurs. Le vote des
        // lectures d'un brin, le code externe et l'empreinte du fichier l'arrêtent.
        for (rate, rates, minimum, wrong_max) in [
            (Rate::HALF, (0.01, 0.01, 0.01), 0.96, 0),
            (Rate::HALF, (0.02, 0.02, 0.02), 0.75, 0),
            (Rate::THIRD, (0.02, 0.02, 0.02), 0.95, 0),
            (Rate::THIRD, (0.033, 0.033, 0.033), 0.80, 1),
            (Rate::QUARTER, (0.033, 0.033, 0.033), 0.93, 2),
            (Rate::QUARTER, (0.05, 0.05, 0.05), 0.55, 2),
            (Rate::THREE_QUARTERS, (0.0033, 0.0033, 0.0033), 0.90, 0),
            (Rate::THREE_QUARTERS, (0.01, 0.01, 0.01), 0.35, 1),
        ] {
            let (solved, wrong) = restored(rate, rates, 200);
            assert!(
                solved as f64 / 200.0 >= minimum,
                "rendement {}, taux {rates:?} : {solved}/200",
                rate.name()
            );
            assert!(
                wrong <= wrong_max,
                "rendement {} : {wrong} messages faux",
                rate.name()
            );
        }
    }

    #[test]
    fn the_salt_turns_an_index_error_into_an_erasure() {
        let layout = Layout::strand(Rate::HALF);
        let mut decoder = Decoder::new(layout);
        // Les vingt-quatre premières bases portent le sel, une par bit.
        for seed in 0..4u64 {
            let free = pseudo_random(layout.free_bytes(), seed);
            let dna = encode(&layout, &free);
            for position in 0..SALT_BITS {
                for replacement in *b"ACG" {
                    if replacement == dna[position] {
                        continue;
                    }
                    let mut read = dna.clone();
                    read[position] = replacement;
                    let decoded = decoder.decode(&read);
                    let truth = decoded.message.as_deref() == Some(free.as_slice());
                    assert!(
                        truth || decoded.message.is_none(),
                        "position {position}, base {} : un index faux passe",
                        dna[position]
                    );
                }
            }
        }
    }

    #[test]
    fn random_reads_are_never_decoded() {
        let mut rng = SeededRng::new(2020, 0x63);
        // Un budget court suffit : une lecture au hasard n'a jamais de chaîne à moins d'une
        // marge d'une autre.
        let search = Search {
            budget: 2000,
            ..Search::default()
        };
        for rate in Rate::ALL {
            for layout in [Layout::strand(rate), Layout::header(rate)] {
                let mut decoder = Decoder::with_search(layout, search);
                for trial in 0..40u64 {
                    let len = layout.chars + (trial % 17) as usize - 8;
                    let read: Vec<u8> = (0..len).map(|_| BASES[rng.below(4) as usize]).collect();
                    let decoded = decoder.decode(&read);
                    assert_ne!(decoded.outcome, Outcome::Solved, "{}", rate.name());
                }
            }
        }
    }

    #[test]
    fn reads_of_another_rate_are_not_decoded() {
        let other = Layout::strand(Rate::HALF);
        let dna = encode(&other, &pseudo_random(other.free_bytes(), 3));
        for rate in [Rate::THIRD, Rate::QUARTER] {
            let decoded = Decoder::new(Layout::strand(rate)).decode(&dna);
            assert_ne!(decoded.outcome, Outcome::Solved);
        }
        let long = encode(
            &Layout::strand(Rate::QUARTER),
            &pseudo_random(Layout::strand(Rate::QUARTER).free_bytes(), 3),
        );
        let decoded = Decoder::new(Layout::strand(Rate::HALF)).decode(&long);
        assert_ne!(decoded.outcome, Outcome::Solved);
    }

    /// Nombre de bases, en fin de brin, qui ne portent que la queue de zéros.
    fn runout_positions_of(layout: &Layout) -> usize {
        let mut rank = 0;
        for position in 0..layout.chars {
            rank += usize::from(layout.rate.bits_at(position));
            if rank == layout.free_from + layout.free_len {
                return layout.chars - position - 1;
            }
        }
        unreachable!("la queue suit les bits libres")
    }

    #[test]
    fn reads_that_end_early_or_late() {
        let layout = Layout::strand(Rate::HALF);
        let free = pseudo_random(layout.free_bytes(), 5);
        let dna = encode(&layout, &free);
        let mut decoder = Decoder::new(layout);

        // Trois bases de trop en fin de lecture : elles sont comptées comme insérées.
        let mut late = dna.clone();
        late.extend_from_slice(b"GTA");
        let decoded = decoder.decode(&late);
        assert_eq!(decoded.outcome, Outcome::Solved);
        assert_eq!(decoded.errors, 3);
        assert_eq!(decoded.message.as_deref(), Some(free.as_slice()));

        // Une base de trop en tête.
        let mut early = b"T".to_vec();
        early.extend_from_slice(&dna);
        let decoded = decoder.decode(&early);
        assert_eq!(decoded.outcome, Outcome::Solved);
        assert_eq!(decoded.message.as_deref(), Some(free.as_slice()));

        // La lecture s'arrête dans la queue de zéros : les bases manquantes sont connues.
        assert_eq!(runout_positions_of(&layout), 16);
        let decoded = decoder.decode(&dna[..layout.chars - 4]);
        assert_eq!(decoded.outcome, Outcome::Solved, "{:?}", decoded.outcome);
        assert_eq!(decoded.message.as_deref(), Some(free.as_slice()));

        // Elle s'arrête dans la charge utile : les derniers bits sont perdus, rien n'est inventé.
        let decoded = decoder.decode(&dna[..layout.chars - 40]);
        assert_ne!(decoded.outcome, Outcome::Solved);

        // Une lecture vide, ou qui n'est pas de l'ADN.
        assert_eq!(decoder.decode(&[]).outcome, Outcome::Lost);
        assert_eq!(decoder.decode(b"ACGN").outcome, Outcome::Invalid);
    }

    #[test]
    fn a_read_cut_inside_the_runout_costs_nothing_more() {
        let layout = Layout::strand(Rate::QUARTER);
        let free = pseudo_random(layout.free_bytes(), 21);
        let dna = encode(&layout, &free);
        let mut decoder = Decoder::new(layout);
        // La queue compte quarante-cinq bases, un contrôle pur compris. Une lecture qui s'arrête
        // dedans — un séquenceur qui s'arrête à 290 bases sur un brin de 300 — est lue sans
        // mal : les bases manquantes sont connues, la lecture ne les a simplement pas lues. Elle
        // doit pourtant avoir lu deux positions de queue : moins, et le dernier bit libre
        // pourrait se deviner — une suppression et une correspondance chanceuse.
        let runout = runout_positions_of(&layout);
        assert_eq!(runout, 45);
        for cut in 1..=runout - 2 {
            let decoded = decoder.decode(&dna[..layout.chars - cut]);
            assert_eq!(decoded.outcome, Outcome::Solved, "{cut} bases de moins");
            assert_eq!(decoded.errors, 0, "{cut} bases de moins");
            assert_eq!(decoded.message.as_deref(), Some(free.as_slice()));
            assert!(
                decoded.hypotheses as usize <= 8 * layout.chars,
                "{cut} bases de moins : {} hypothèses",
                decoded.hypotheses
            );
        }
        // Arrêtée au ras de la charge utile, ou dedans : les derniers bits ne sont pas assez
        // vérifiés, ou pas lus. La lecture est déclarée perdue, sans chercher.
        for cut in runout - 1..=runout + 40 {
            let decoded = decoder.decode(&dna[..layout.chars - cut]);
            assert_eq!(
                (decoded.outcome, decoded.hypotheses),
                (Outcome::Lost, 1),
                "{cut} bases de moins"
            );
        }
        // Une base de moins dans la charge utile, mais assez loin pour que la recherche soit
        // lancée : elle ne trouve rien.
        let decoded = decoder.decode(&dna[..layout.chars - runout - 41]);
        assert_eq!(decoded.outcome, Outcome::Lost);

        // Trop courte ou trop longue pour la ligne d'abandon : rien ne peut aboutir, la lecture
        // est refusée sans chercher.
        let short = decoder.decode(&dna[..layout.chars / 2]);
        assert_eq!((short.outcome, short.hypotheses), (Outcome::Lost, 1));
        let mut long = dna.clone();
        long.resize(2 * layout.chars, b'A');
        let long = decoder.decode(&long);
        assert_eq!((long.outcome, long.hypotheses), (Outcome::Lost, 1));
    }

    /// Somme de contrôle de ce que la recherche rend de lectures reproductibles : l'issue, le
    /// score, les erreurs, le nombre d'hypothèses et le message de chaque lecture.
    fn checksum(rate: Rate, each: f64, budget: usize, trials: usize) -> u64 {
        let layout = Layout::strand(rate);
        let search = Search {
            budget,
            ..Search::default()
        };
        let mut rng = SeededRng::new(0xC0FFEE, u64::from(rate.code()));
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        let mix = |hash: &mut u64, byte: u8| {
            *hash ^= u64::from(byte);
            *hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        };
        for _ in 0..trials {
            let mut free = vec![0u8; layout.free_bytes()];
            rng.fill_bytes(&mut free);
            let dna = encode(&layout, &free);
            let read = damage(&dna, (each, each, each), &mut rng);
            let decoded = decode(&layout, &read, search);
            let outcome = match decoded.outcome {
                Outcome::Solved => 1,
                Outcome::Lost => 2,
                Outcome::Exhausted => 3,
                Outcome::Rival => 4,
                Outcome::Invalid => 5,
            };
            mix(&mut hash, outcome);
            for array in [
                decoded.score.to_le_bytes(),
                decoded.hypotheses.to_le_bytes(),
                u32::from(decoded.errors).to_le_bytes(),
            ] {
                for byte in array {
                    mix(&mut hash, byte);
                }
            }
            for &byte in decoded.message.as_deref().unwrap_or(&[]) {
                mix(&mut hash, byte);
            }
        }
        hash
    }

    /// L'ordre dans lequel la recherche développe ses hypothèses fait partie de ses résultats :
    /// quelle chaîne gagne, à quel prix. Ces sommes figent le décodage de lectures
    /// reproductibles, pour qu'un changement de la recherche — une file plus rapide, un état plus
    /// compact — ne le change pas à son insu.
    #[test]
    fn the_search_develops_its_hypotheses_in_a_pinned_order() {
        let points = [
            (Rate::QUARTER, 0.033),
            (Rate::QUARTER, 0.05),
            (Rate::THIRD, 0.02),
            (Rate::THIRD, 0.04),
            (Rate::HALF, 0.01),
            (Rate::HALF, 0.02),
            (Rate::THREE_QUARTERS, 0.005),
            (Rate::THREE_QUARTERS, 0.015),
        ];
        let found: Vec<u64> = points
            .iter()
            .map(|&(rate, each)| checksum(rate, each, 2_500, 250))
            .collect();
        assert_eq!(
            // Mesurées sur la recherche de référence : un tas binaire, un état de 32 bits
            // pour les seize dernières bases. À recalculer délibérément, jamais par accident.
            found,
            [
                0x32a2196d45abb942,
                0xe0d6cdf2dfc77eef,
                0xca788ee62a5a75df,
                0x22ba802015e2d940,
                0xd5c69f0e75ed2fb4,
                0x43d5f2e2ca9a45be,
                0x46b4d7321d929d7c,
                0x185525d562be4c5d,
            ],
            "sommes de contrôle de la recherche : {found:#018x?}"
        );

        // Même chose au-delà du seuil où le tas cède la place aux seaux : à égalité de score, les
        // hypothèses doivent y garder l'ordre de leur rang.
        let spills = [
            (Rate::QUARTER, 0.01),
            (Rate::THIRD, 0.01),
            (Rate::HALF, 0.01),
            (Rate::THREE_QUARTERS, 0.005),
        ];
        let found: Vec<u64> = spills
            .iter()
            .map(|&(rate, each)| checksum(rate, each, 5_000, 200))
            .collect();
        assert_eq!(
            found,
            // Mesurées une fois la recherche rendue identique à la précédente, lecture après
            // lecture, sur 64 000 lectures.
            [
                0xc558cdb3718ff780,
                0xa0db90b0babe9753,
                0x171a1818d984896d,
                0xfef1486b7612b4f6,
            ],
            "sommes de contrôle passé le seuil : {found:#018x?}"
        );
    }

    /// La base suivante dans l'ordre A, C, G, T.
    fn next_base(base: u8) -> u8 {
        let rank = BASES.iter().position(|&known| known == base).unwrap_or(0);
        BASES[(rank + 1) % 4]
    }

    #[test]
    fn a_rival_message_makes_the_decoding_fail() {
        // Un historique d'un seul bit et une queue de quatre : un bit retourné en fin de message
        // se rattrape dès la position suivante, et un autre message explique la lecture aussi
        // bien que le vrai. Le décodeur ne doit pas choisir.
        let short = Rate {
            pattern: &[1],
            chars: 60,
            history: 1,
            reward: -127,
            error_per_mille: 150,
        };
        let layout = Layout {
            rate: short,
            chars: short.chars,
            salt: None,
            free_from: SALT_BITS,
            free_len: 32,
        };
        let free = pseudo_random(layout.free_bytes(), 9);
        let dna = encode(&layout, &free);
        let mut decoder = Decoder::new(layout);
        // Dernière base libre : le rival se rattrape sur la queue forcée, et finit à égalité, ou
        // une erreur devant. Il est vu, et le décodage échoue au lieu de choisir.
        let mut read = dna.clone();
        read[55] = next_base(read[55]);
        let decoded = decoder.decode(&read);
        assert_eq!(decoded.outcome, Outcome::Rival);
        assert!(decoded.message.is_none());

        // Sans faute, le message reste seul.
        let decoded = decoder.decode(&dna);
        assert_eq!(decoded.outcome, Outcome::Solved);
        assert_eq!(decoded.message.as_deref(), Some(free.as_slice()));

        // Avec les douze bits d'historique du format à ⅓, une base remplacée dans la charge
        // utile se corrige, ou fait un effacement : jamais un message faux.
        let layout = Layout::strand(Rate::THIRD);
        let free = pseudo_random(layout.free_bytes(), 9);
        let dna = encode(&layout, &free);
        let mut decoder = Decoder::new(layout);
        let mut solved = 0;
        for position in SALT_BITS..layout.chars - 40 {
            for base in *b"ACGT" {
                if base == dna[position] {
                    continue;
                }
                let mut read = dna.clone();
                read[position] = base;
                let decoded = decoder.decode(&read);
                let truth = decoded.message.as_deref() == Some(free.as_slice());
                assert!(
                    truth || decoded.message.is_none(),
                    "position {position} : un message faux passe"
                );
                solved += usize::from(truth);
            }
        }
        assert!(
            solved >= 3 * (layout.chars - 40 - SALT_BITS),
            "{solved} lectures restituées"
        );
    }

    #[test]
    fn a_read_cut_by_the_budget_never_yields_a_message() {
        // Un budget trop court ne laisse rien passer : la chaîne qui finit, si elle finit, n'a
        // plus droit au bénéfice du doute.
        let layout = Layout::strand(Rate::HALF);
        let free = pseudo_random(layout.free_bytes(), 13);
        let dna = encode(&layout, &free);
        let mut rng = SeededRng::new(21, 0x66);
        let mut solved = 0;
        for budget in [200, 400, 800, 1600, 3200, 6400, 12_800, 25_600] {
            let search = Search {
                budget,
                ..Search::default()
            };
            let mut decoder = Decoder::with_search(layout, search);
            for _ in 0..40 {
                let read = damage(&dna, (0.02, 0.02, 0.02), &mut rng);
                let decoded = decoder.decode(&read);
                let truth = decoded.message.as_deref() == Some(free.as_slice());
                assert!(
                    truth || decoded.message.is_none(),
                    "budget {budget} : {:?}",
                    decoded.outcome
                );
                solved += usize::from(truth);
            }
        }
        // Les budgets les plus larges restituent une part de ces lectures à 6 % d'erreurs.
        assert!(solved >= 20, "{solved} lectures restituées");
    }

    #[test]
    fn the_budget_decides_of_a_hard_read() {
        let layout = Layout::strand(Rate::HALF);
        let free = pseudo_random(layout.free_bytes(), 11);
        let dna = encode(&layout, &free);
        let mut rng = SeededRng::new(8, 0x65);
        let read = damage(&dna, (0.03, 0.03, 0.03), &mut rng);
        let solved = Decoder::new(layout).decode(&read);
        let starving = Search {
            budget: 40,
            ..Search::default()
        };
        let failed = Decoder::with_search(layout, starving).decode(&read);
        assert_eq!(failed.outcome, Outcome::Exhausted);
        // Ce n'est pas la lecture qui est en cause : c'est le budget.
        assert_ne!(solved.outcome, Outcome::Exhausted);
    }
}
