//! Code LT (Luby Transform) : structure des gouttes, épluchage, décodabilité.
//!
//! Une goutte est le OU exclusif de quelques blocs du corps du fichier. Sa structure (degré et
//! indices des blocs) dépend uniquement de sa graine : l'encodeur et le décodeur la recalculent
//! avec le même générateur, ce qui dispense de la transmettre.
//!
//! L'épluchage est insensible à l'ordre d'arrivée : il résout l'ensemble maximal de blocs que
//! la structure permet, quel que soit l'ordre. Les fonctions travaillent sur des tableaux
//! plats, sans allocation par goutte : une archive de plusieurs méga-octets relie plusieurs
//! millions de blocs.

use crate::codec::rng::SeededRng;
use crate::codec::soliton::RobustSoliton;
use crate::error::{DnaError, Result};
use std::collections::VecDeque;

/// Numéro du flux de tirage du degré.
pub const DEGREE_STREAM: u64 = 0x0000_0000_0000_0001;
/// Numéro du flux de tirage des indices (différent du précédent).
pub const INDEX_STREAM: u64 = 0x0000_0000_0000_0002;

/// Valeurs par défaut des paramètres de la distribution soliton robuste (article DNA Fountain).
pub const DEFAULT_C: f64 = 0.025;
pub const DEFAULT_DELTA: f64 = 0.001;

/// Calcule la structure des gouttes pour un nombre de blocs donné.
pub struct LayoutSampler {
    k: u32,
    soliton: RobustSoliton,
    /// Marqueur par bloc : `marks[i] == generation` signifie « tiré pour la goutte en cours ».
    marks: Vec<u32>,
    generation: u32,
}

impl LayoutSampler {
    /// Prépare l'échantillonnage pour `k` blocs.
    pub fn new(k: u32, c: f64, delta: f64) -> Result<Self> {
        let soliton = RobustSoliton::new(k, c, delta)?;
        Ok(Self {
            k,
            soliton,
            marks: vec![0; k as usize],
            generation: 1,
        })
    }

    /// Nombre de blocs.
    pub fn k(&self) -> u32 {
        self.k
    }

    /// Degré moyen d'une goutte.
    pub fn mean_degree(&self) -> f64 {
        self.soliton.mean()
    }

    /// Écrit dans `indices` (triés) les blocs de la goutte de graine `seed` et retourne son degré.
    pub fn layout(&mut self, seed: u32, indices: &mut Vec<u32>) -> u32 {
        let mut rng = SeededRng::new(u64::from(seed), DEGREE_STREAM);
        let degree = self.soliton.sample(&mut rng);

        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.marks.fill(0);
            self.generation = 1;
        }
        let generation = self.generation;

        // Algorithme de Floyd : `degree` indices distincts, chaque tirage dans [0, j].
        let mut index_rng = SeededRng::new(u64::from(seed), INDEX_STREAM);
        indices.clear();
        indices.reserve(degree as usize);
        for j in self.k - degree..self.k {
            let candidate = index_rng.below(u64::from(j) + 1) as u32;
            if self.marks[candidate as usize] == generation {
                indices.push(j);
                self.marks[j as usize] = generation;
            } else {
                indices.push(candidate);
                self.marks[candidate as usize] = generation;
            }
        }
        indices.sort_unstable();
        degree
    }
}

/// Calcule dans `out` le OU exclusif de plusieurs blocs de longueur `chunk_len` (multiple de 8
/// octets), vus comme des mots de 64 bits. `scratch` sert d'accumulateur (longueur
/// `chunk_len / 8`).
pub fn xor_chunks(
    words: &[u64],
    words_per_chunk: usize,
    indices: &[u32],
    scratch: &mut [u64],
    out: &mut [u8],
) {
    scratch.fill(0);
    for &index in indices {
        let base = index as usize * words_per_chunk;
        for (slot, &word) in scratch.iter_mut().zip(&words[base..base + words_per_chunk]) {
            *slot ^= word;
        }
    }
    for (chunk, &value) in out.as_chunks_mut::<8>().0.iter_mut().zip(scratch.iter()) {
        chunk.copy_from_slice(&value.to_le_bytes());
    }
}

fn check_indices(indices: &[u32], k: u32) -> Result<()> {
    if k == 0 {
        return Err(DnaError::Decoding("aucun bloc à reconstituer".to_string()));
    }
    if let Some(&bad) = indices.iter().find(|&&index| index >= k) {
        return Err(DnaError::Decoding(format!(
            "indice de bloc {bad} hors des {k} blocs du corps"
        )));
    }
    Ok(())
}

/// Suivi incrémental de la décodabilité, sur la structure seule.
///
/// L'encodeur y ajoute ses gouttes au fil de la production et sait ainsi, à tout moment, si
/// l'archive est décodable. Les valeurs des blocs n'y servent à rien : seul compte l'ensemble
/// des blocs que chaque goutte relie.
pub struct StructureTracker {
    k: u32,
    /// Blocs résolus.
    solved: Vec<bool>,
    solved_count: u32,
    /// Degré restant de chaque goutte : nombre de ses blocs non résolus.
    degree: Vec<u32>,
    /// Indices de toutes les gouttes, bout à bout ; `spans[id]` délimite ceux de la goutte.
    indices: Vec<u32>,
    spans: Vec<u32>,
    /// Pour chaque bloc, les gouttes qui le relient encore.
    adjacency: Vec<Vec<u32>>,
    /// Gouttes de degré 1 en attente de traitement.
    queue: VecDeque<u32>,
}

impl StructureTracker {
    /// Prépare le suivi de `k` blocs.
    pub fn new(k: u32) -> Result<Self> {
        if k == 0 {
            return Err(DnaError::InvalidConfig(
                "le nombre de blocs doit être au moins 1".to_string(),
            ));
        }
        Ok(Self {
            k,
            solved: vec![false; k as usize],
            solved_count: 0,
            degree: Vec::new(),
            indices: Vec::new(),
            spans: vec![0],
            adjacency: vec![Vec::new(); k as usize],
            queue: VecDeque::new(),
        })
    }

    /// Nombre de blocs à reconstituer.
    pub fn k(&self) -> u32 {
        self.k
    }

    /// Nombre de blocs résolus.
    pub fn solved(&self) -> u32 {
        self.solved_count
    }

    /// Vrai quand tous les blocs sont résolus.
    pub fn is_complete(&self) -> bool {
        self.solved_count == self.k
    }

    /// Ajoute une goutte et poursuit l'épluchage. Sans effet une fois complet.
    ///
    /// Les indices doivent être distincts et inférieurs au nombre de blocs.
    pub fn add(&mut self, indices: &[u32]) -> Result<()> {
        check_indices(indices, self.k)?;
        if indices.is_empty() || self.is_complete() {
            return Ok(());
        }
        // Réduire la goutte contre les blocs déjà résolus.
        let start = self.indices.len();
        self.indices.extend(
            indices
                .iter()
                .copied()
                .filter(|&index| !self.solved[index as usize]),
        );
        let end = self.indices.len();
        let remaining = (end - start) as u32;
        if remaining == 0 {
            self.indices.truncate(start);
            return Ok(());
        }
        let id = self.degree.len() as u32;
        self.degree.push(remaining);
        self.spans.push(end as u32);
        for &index in &self.indices[start..end] {
            self.adjacency[index as usize].push(id);
        }
        if remaining == 1 {
            self.queue.push_back(id);
        }
        self.peel();
        if self.is_complete() {
            self.adjacency.clear();
        }
        Ok(())
    }

    /// Résout itérativement les gouttes de degré 1.
    fn peel(&mut self) {
        while let Some(id) = self.queue.pop_front() {
            if self.degree[id as usize] != 1 {
                continue;
            }
            // L'unique bloc non résolu de la goutte.
            let span = self.spans[id as usize] as usize..self.spans[id as usize + 1] as usize;
            let Some(&chunk) = self.indices[span]
                .iter()
                .find(|&&c| !self.solved[c as usize])
            else {
                continue;
            };
            self.solved[chunk as usize] = true;
            self.solved_count += 1;
            let linked = std::mem::take(&mut self.adjacency[chunk as usize]);
            for other in linked {
                if other == id {
                    continue;
                }
                let degree = &mut self.degree[other as usize];
                *degree -= 1;
                if *degree == 1 {
                    self.queue.push_back(other);
                }
            }
        }
    }
}

/// Résultat d'un épluchage par lot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeelOutcome {
    /// Blocs reconstitués.
    pub solved: u32,
    /// Le corps du fichier, `k × chunk_len` octets, si tous les blocs sont reconstitués.
    pub body: Option<Vec<u8>>,
}

impl PeelOutcome {
    /// Vrai si tous les blocs sont reconstitués.
    pub fn is_complete(&self) -> bool {
        self.body.is_some()
    }
}

/// Le graphe des gouttes d'une archive, en tableaux compacts, prêt à éplucher.
///
/// Le construire une fois, puis l'éplucher autant de fois que nécessaire : l'encodeur y rejoue
/// des centaines de tirages de perte, et chaque épluchage n'alloue plus que son propre état —
/// deux compteurs par goutte et un booléen par bloc — au lieu du graphe entier.
pub struct PeelingGraph {
    k: usize,
    count: usize,
    /// `offset[id]..offset[id + 1]` délimite, dans `flat`, les blocs de la goutte `id`.
    offset: Vec<u32>,
    flat: Vec<u32>,
    /// `chunk_offset[bloc]..chunk_offset[bloc + 1]` délimite, dans `linked`, les gouttes qui
    /// relient `bloc`.
    chunk_offset: Vec<u32>,
    linked: Vec<u32>,
}

impl PeelingGraph {
    /// Prépare l'épluchage de `layouts` — indices des blocs de chaque goutte, distincts et
    /// inférieurs à `k` — pour `k` blocs.
    pub fn new(layouts: &[&[u32]], k: u32) -> Result<Self> {
        if k == 0 {
            return Err(DnaError::Decoding("aucun bloc à reconstituer".to_string()));
        }
        for layout in layouts {
            check_indices(layout, k)?;
        }

        // Indices de toutes les gouttes, bout à bout.
        let count = layouts.len();
        let mut offset = vec![0u32; count + 1];
        for (id, layout) in layouts.iter().enumerate() {
            let end = offset[id] + layout.len() as u32;
            offset[id + 1] = end;
        }
        let total = offset[count] as usize;
        let mut flat = Vec::with_capacity(total);
        for layout in layouts {
            flat.extend_from_slice(layout);
        }

        // Gouttes rattachées à chaque bloc, en représentation compacte.
        let chunks = k as usize;
        let mut chunk_offset = vec![0u32; chunks + 1];
        for &chunk in &flat {
            chunk_offset[chunk as usize + 1] += 1;
        }
        for index in 1..=chunks {
            chunk_offset[index] += chunk_offset[index - 1];
        }
        let mut linked = vec![0u32; total];
        let mut cursor = chunk_offset[..chunks].to_vec();
        for (id, span) in offset.windows(2).enumerate() {
            for &chunk in &flat[span[0] as usize..span[1] as usize] {
                let slot = &mut cursor[chunk as usize];
                linked[*slot as usize] = id as u32;
                *slot += 1;
            }
        }

        Ok(Self {
            k: chunks,
            count,
            offset,
            flat,
            chunk_offset,
            linked,
        })
    }

    /// Épluche, avec les gouttes que retient `keep`.
    ///
    /// `keep` reçoit chaque goutte, par son identifiant et dans l'ordre : elle répond si la
    /// goutte participe. Avec `chunk_len` nul, seules les structures sont lues : l'épluchage dit
    /// alors si les gouttes retenues suffisent à reconstituer le corps, sans s'occuper des
    /// valeurs. Sinon, `payloads` contient la charge utile des gouttes, bout à bout, `chunk_len`
    /// octets chacune, et sert d'espace de travail : il est modifié.
    pub fn peel(
        &self,
        mut keep: impl FnMut(usize) -> bool,
        payloads: &mut [u8],
        chunk_len: usize,
    ) -> PeelOutcome {
        let count = self.count;
        // Degré restant de chaque goutte ; 0 pour une goutte écartée.
        let mut degree = vec![0u32; count];
        let mut solved = vec![false; self.k];
        let mut solved_count = 0usize;
        let with_values = chunk_len > 0;
        let mut values = vec![0u8; if with_values { self.k * chunk_len } else { 0 }];
        let mut queue: Vec<u32> = Vec::new();

        // Les degrés, goutte après goutte ; celles que `keep` écarte restent à zéro.
        for (id, degree) in degree.iter_mut().enumerate() {
            if keep(id) {
                *degree = self.offset[id + 1] - self.offset[id];
                if *degree == 1 {
                    queue.push(id as u32);
                }
            }
        }

        while let Some(id) = queue.pop() {
            if degree[id as usize] != 1 {
                continue;
            }
            let span = self.offset[id as usize] as usize..self.offset[id as usize + 1] as usize;
            // L'unique bloc non résolu de la goutte.
            let Some(&chunk) = self.flat[span]
                .iter()
                .find(|&&chunk| !solved[chunk as usize])
            else {
                continue;
            };
            let chunk = chunk as usize;
            solved[chunk] = true;
            solved_count += 1;
            if with_values {
                let value = &mut values[chunk * chunk_len..][..chunk_len];
                value.copy_from_slice(&payloads[id as usize * chunk_len..][..chunk_len]);
            }
            if solved_count == self.k {
                break;
            }
            for &other in &self.linked
                [self.chunk_offset[chunk] as usize..self.chunk_offset[chunk + 1] as usize]
            {
                if other == id || degree[other as usize] == 0 {
                    continue;
                }
                let degree = &mut degree[other as usize];
                *degree -= 1;
                if *degree == 1 {
                    queue.push(other);
                } else if *degree == 0 {
                    continue;
                }
                if with_values {
                    // La goutte perd ce bloc : sa valeur lui est retirée.
                    let target = &mut payloads[other as usize * chunk_len..][..chunk_len];
                    for (slot, &byte) in target
                        .iter_mut()
                        .zip(&values[chunk * chunk_len..][..chunk_len])
                    {
                        *slot ^= byte;
                    }
                }
            }
        }

        PeelOutcome {
            solved: solved_count as u32,
            body: (solved_count == self.k).then_some(values),
        }
    }

    /// Les gouttes que retient `keep` suffisent-elles à reconstituer le corps ? Structure seule.
    pub fn solves(&self, keep: impl FnMut(usize) -> bool) -> bool {
        self.peel(keep, &mut [], 0).solved as usize == self.k
    }
}

/// Reconstitue les blocs d'un corps par épluchage.
///
/// - `layouts` : indices des blocs reliés par chaque goutte (distincts, inférieurs à `k`) ;
/// - `payloads` : charge utile des gouttes, bout à bout, `chunk_len` octets chacune ; le
///   contenu est utilisé comme espace de travail et modifié.
///
/// Avec `chunk_len` nul, seules les structures sont lues : la fonction dit alors si l'ensemble
/// des gouttes suffit à reconstituer le corps, sans s'occuper des valeurs.
pub fn peel(
    layouts: &[&[u32]],
    payloads: &mut [u8],
    k: u32,
    chunk_len: usize,
) -> Result<PeelOutcome> {
    let graph = PeelingGraph::new(layouts, k)?;
    if payloads.len() != layouts.len() * chunk_len {
        return Err(DnaError::Decoding(format!(
            "{} octets de charge utile pour {} gouttes de {} octets",
            payloads.len(),
            layouts.len(),
            chunk_len
        )));
    }
    Ok(graph.peel(|_| true, payloads, chunk_len))
}

/// L'épluchage aboutit-il sur cet ensemble de gouttes ? Structure seule.
pub fn peels(layouts: &[&[u32]], k: u32) -> bool {
    PeelingGraph::new(layouts, k).is_ok_and(|graph| graph.solves(|_| true))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(body: &[u8]) -> Vec<u64> {
        body.as_chunks::<8>()
            .0
            .iter()
            .map(|c| u64::from_le_bytes(*c))
            .collect()
    }

    /// Gouttes aléatoires sur un corps, avec leurs charges utiles.
    fn droplets(k: u32, count: usize, seed: u64) -> (Vec<u8>, Vec<Vec<u32>>, Vec<u8>) {
        let body: Vec<u8> = (0..k as usize * 32).map(|i| (i * 7 % 251) as u8).collect();
        let w = words(&body);
        let mut sampler = LayoutSampler::new(k, 0.05, 0.001).unwrap();
        let mut rng = SeededRng::new(seed, 77);
        let mut indices = Vec::new();
        let mut payload = vec![0u8; 32];
        let mut layouts = Vec::with_capacity(count);
        let mut payloads = Vec::with_capacity(count * 32);
        for _ in 0..count {
            sampler.layout(rng.next_u32(), &mut indices);
            xor_chunks(&w, 4, &indices, &mut [0u64; 4], &mut payload);
            layouts.push(indices.clone());
            payloads.extend_from_slice(&payload);
        }
        (body, layouts, payloads)
    }

    fn references(layouts: &[Vec<u32>]) -> Vec<&[u32]> {
        layouts.iter().map(Vec::as_slice).collect()
    }

    #[test]
    fn layout_is_deterministic_and_in_range() {
        let mut sampler = LayoutSampler::new(1_000, 0.025, 0.001).unwrap();
        let mut a = Vec::new();
        let mut b = Vec::new();
        for seed in 0..2_000u32 {
            let d1 = sampler.layout(seed, &mut a);
            let d2 = sampler.layout(seed, &mut b);
            assert_eq!(d1, d2);
            assert_eq!(a, b);
            assert_eq!(a.len(), d1 as usize);
            assert!(
                a.windows(2).all(|w| w[0] < w[1]),
                "indices distincts et triés"
            );
            assert!(a.iter().all(|&i| i < 1_000));
        }
    }

    #[test]
    fn layout_covers_all_blocks_in_low_degree() {
        let k = 64u32;
        let mut sampler = LayoutSampler::new(k, 0.025, 0.001).unwrap();
        let mut indices = Vec::new();
        let mut seen = vec![false; k as usize];
        for seed in 0..20_000u32 {
            let degree = sampler.layout(seed, &mut indices);
            if degree <= 2 {
                for &i in &indices {
                    seen[i as usize] = true;
                }
            }
        }
        assert!(seen.iter().all(|&s| s));
    }

    #[test]
    fn xor_chunks_matches_naive() {
        let body: Vec<u8> = (0..96u8).cycle().take(32 * 5).collect();
        let w = words(&body);
        let indices = vec![0u32, 3, 4];
        let mut out = vec![0u8; 32];
        xor_chunks(&w, 4, &indices, &mut [0u64; 4], &mut out);
        let mut naive = vec![0u8; 32];
        for &i in &indices {
            for (slot, &byte) in naive.iter_mut().zip(&body[i as usize * 32..][..32]) {
                *slot ^= byte;
            }
        }
        assert_eq!(out, naive);
        let mut out = vec![0xFF; 16];
        xor_chunks(&[], 2, &[], &mut [0u64; 2], &mut out);
        assert_eq!(out, vec![0u8; 16]);
    }

    #[test]
    fn tracker_is_insensitive_to_arrival_order() {
        for trial in 0..6u64 {
            let (body, layouts, payloads) = droplets(400, 1_500, trial);
            let mut order: Vec<usize> = (0..layouts.len()).collect();
            let mut rng = SeededRng::new(trial, 99);
            for i in (1..order.len()).rev() {
                order.swap(i, rng.below(i as u64 + 1) as usize);
            }
            let mut tracker = StructureTracker::new(400).unwrap();
            let mut used = 0;
            for &position in &order {
                tracker.add(&layouts[position]).unwrap();
                used += 1;
                if tracker.is_complete() {
                    break;
                }
            }
            assert!(tracker.is_complete(), "essai {trial}");
            assert_eq!(tracker.solved(), 400);

            // Le lot des gouttes retenues reconstitue le corps exact.
            let kept: Vec<&[u32]> = order[..used]
                .iter()
                .map(|&p| layouts[p].as_slice())
                .collect();
            let mut flat = Vec::new();
            for &p in &order[..used] {
                flat.extend_from_slice(&payloads[p * 32..][..32]);
            }
            let outcome = peel(&kept, &mut flat, 400, 32).unwrap();
            assert_eq!(outcome.body.as_deref(), Some(body.as_slice()));
            assert!(peels(&kept, 400));
        }
    }

    #[test]
    fn a_late_droplet_is_reduced_against_solved_chunks() {
        // Trois blocs ; la goutte « a xor b » arrive après la résolution de a : elle doit
        // être réduite, puis résoudre b, ce qui réduit « b xor c » et résout c.
        let mut tracker = StructureTracker::new(3).unwrap();
        tracker.add(&[1, 2]).unwrap();
        tracker.add(&[0]).unwrap();
        assert_eq!(tracker.solved(), 1);
        tracker.add(&[0, 1]).unwrap();
        assert_eq!(tracker.solved(), 3);
        assert!(tracker.is_complete());
        // Plus rien à faire.
        tracker.add(&[2]).unwrap();
        assert_eq!(tracker.solved(), 3);
    }

    #[test]
    fn peel_rebuilds_exact_values() {
        let (body, layouts, payloads) = droplets(2_000, 5_000, 5);
        let outcome = peel(&references(&layouts), &mut payloads.clone(), 2_000, 32).unwrap();
        assert_eq!(outcome.body.as_deref(), Some(body.as_slice()));
        // Amputé de 20 % des gouttes prises au hasard, il reste deux fois trop de gouttes :
        // l'ensemble décode toujours.
        let mut rng = SeededRng::new(5, 6);
        for _ in 0..20 {
            let kept: Vec<&[u32]> = layouts
                .iter()
                .filter(|_| rng.next_f64() < 0.8)
                .map(Vec::as_slice)
                .collect();
            assert!(peels(&kept, 2_000));
        }
    }

    #[test]
    fn incomplete_peeling_is_reported_never_guessed() {
        let (body, layouts, mut payloads) = droplets(50, 20, 9);
        let outcome = peel(&references(&layouts), &mut payloads, 50, 32).unwrap();
        assert!(outcome.body.is_none());
        assert!(outcome.solved < 50);
        assert!(!outcome.is_complete());
        assert!(!peels(&references(&layouts), 50));
        assert!(peel(&references(&layouts), &mut payloads, 50, 32)
            .unwrap()
            .body
            .is_none());
        assert!(!peel(&[&[0]], &mut [0; 32], 2, 32).unwrap().is_complete());
        assert_eq!(body.len(), 50 * 32);
        // Cas limites : un cycle sans goutte de degré 1 ne démarre jamais, et un bloc que
        // aucune goutte ne relie n'est jamais résolu.
        assert!(!peels(&[], 5));
        assert!(peels(&[&[0, 1], &[1, 2], &[0], &[2]], 3));
        assert!(peels(&[&[0, 1], &[1, 2], &[0]], 3));
        assert!(!peels(&[&[0, 1], &[1, 2], &[0]], 4));
        assert!(!peels(&[&[0, 1], &[1, 2], &[0, 2]], 3));
        // Un doublon n'est jamais résolu : il n'a pas le sens d'une goutte.
        assert!(!peels(&[&[0, 0]], 1));
    }

    #[test]
    fn malformed_inputs_are_rejected() {
        assert!(peel(&[&[7]], &mut [], 5, 0).is_err());
        assert!(peel(&[], &mut [], 0, 0).is_err());
        assert!(peel(&[&[0]], &mut [1, 2, 3], 1, 2).is_err());
        let mut tracker = StructureTracker::new(4).unwrap();
        assert!(tracker.add(&[4]).is_err());
        assert!(StructureTracker::new(0).is_err());
    }

    #[test]
    fn a_graph_can_be_peeled_many_times_keeping_some_droplets() {
        let layouts: Vec<Vec<u32>> = vec![vec![0, 1], vec![1, 2], vec![0], vec![2]];
        let references: Vec<&[u32]> = layouts.iter().map(Vec::as_slice).collect();
        let graph = PeelingGraph::new(&references, 3).unwrap();

        // Toutes les gouttes : oui. Sans la dernière : oui encore.
        assert!(graph.solves(|_| true));
        assert!(graph.solves(|id| id < 3));
        // Seules les deux gouttes de degré 2 : le décollage manque.
        assert!(!graph.solves(|id| id < 2));
        assert!(!graph.solves(|_| false));

        // Le même graphe redonne les valeurs, et l'épluchage n'a rien modifié.
        let mut payloads: Vec<u8> = (0..4 * 32).map(|i| i as u8).collect();
        let mut kept: Vec<u8> = payloads.clone();
        let outcome = graph.peel(|id| id < 2, &mut kept, 32);
        assert_eq!(outcome.solved, 0, "sans goutte de degré 1, rien ne démarre");
        let complete = graph.peel(|_| true, &mut payloads, 32);
        assert!(complete.is_complete());
        let expected = peel(
            &references,
            &mut (0..4 * 32).map(|i| i as u8).collect::<Vec<u8>>(),
            3,
            32,
        )
        .unwrap();
        assert_eq!(complete.body, expected.body);
    }
}
