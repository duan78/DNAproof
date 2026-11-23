//! Reed-Solomon sur les corps à 47 et à 47² éléments.
//!
//! Deux corps : GF(47), pour le code interne d'un brin grass (un symbole par trimère), et son
//! extension quadratique GF(47²), les polynômes `a + b·x` à coefficients dans GF(47) pris
//! modulo `x² − 5`, pour le code externe d'un bloc de 713 brins : un code de Reed-Solomon sur
//! GF(47) ne dépasse pas 46 symboles.
//!
//! Un bloc codé s'écrit `données || parité`. Il corrige tout motif de
//! `2 × erreurs + effacements <= n - k`, les effacements étant des positions connues.
//!
//! Le code est un code d'évaluation : un message est un polynôme de degré inférieur à `k`,
//! évalué aux points α⁰, α¹, … (α engendre le groupe multiplicatif). Le décodage suit Gao
//! (2002) : une interpolation rationnelle par algorithme d'Euclide étendu, sans recherche de
//! racines ni formule de Forney. Les effacements sont les points que l'interpolation ignore.
//!
//! Quand un même jeu d'effacements sert à plusieurs colonnes — les colonnes de charge utile
//! d'un bloc de brins —, un résolveur d'effacements les rétablit par interpolation directe, et
//! ne retombe sur Gao que si une valeur conservée est fausse.
//!
//! Au-delà de sa capacité, un code de Reed-Solomon ne promet pas d'échouer : il peut rendre un
//! autre mot de code valide. L'empreinte SHA-256 de l'enveloppe reste l'arbitre.

use crate::error::{DnaError, Result};

/// Corps fini commutatif, pour les codes d'évaluation.
pub trait Field: Copy + PartialEq + Eq + std::fmt::Debug {
    /// Nombre d'éléments du corps. Un code compte au plus `ORDER − 1` symboles.
    const ORDER: usize;
    /// Un générateur du groupe multiplicatif.
    const GENERATOR: Self;

    fn zero() -> Self;
    fn one() -> Self;
    fn is_zero(self) -> bool;
    fn add(self, other: Self) -> Self;
    fn sub(self, other: Self) -> Self;
    fn mul(self, other: Self) -> Self;
    /// Inverse d'un élément non nul ; zéro pour zéro.
    fn inv(self) -> Self;

    fn div(self, other: Self) -> Self {
        self.mul(other.inv())
    }

    /// Puissance `exponent`-ième.
    fn pow(self, mut exponent: usize) -> Self {
        let mut result = Self::one();
        let mut base = self;
        while exponent > 0 {
            if exponent & 1 == 1 {
                result = result.mul(base);
            }
            base = base.mul(base);
            exponent >>= 1;
        }
        result
    }
}

/// GF(47) : les entiers modulo 47, portés par un octet de 0 à 46.
pub mod gf47 {
    use super::Field;

    /// Nombre d'éléments ; 47 est premier.
    pub const ORDER: usize = 47;
    /// Racine primitive modulo 47 : ses puissances parcourent les 46 éléments non nuls.
    pub const GENERATOR: u8 = 5;

    /// Tables de l'addition, de la multiplication et de l'inverse, indicées par `a · 47 + b`.
    pub(super) const TABLES: ([u8; ORDER * ORDER], [u8; ORDER * ORDER], [u8; ORDER]) = build();

    const fn build() -> ([u8; ORDER * ORDER], [u8; ORDER * ORDER], [u8; ORDER]) {
        let mut add = [0u8; ORDER * ORDER];
        let mut mul = [0u8; ORDER * ORDER];
        let mut inv = [0u8; ORDER];
        let mut a = 0usize;
        while a < ORDER {
            let mut b = 0usize;
            while b < ORDER {
                add[a * ORDER + b] = ((a + b) % ORDER) as u8;
                mul[a * ORDER + b] = ((a * b) % ORDER) as u8;
                if a > 0 && b > 0 && (a * b) % ORDER == 1 {
                    inv[a] = b as u8;
                }
                b += 1;
            }
            a += 1;
        }
        (add, mul, inv)
    }

    /// Un élément de GF(47).
    #[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
    pub struct Gf47(pub u8);

    impl Gf47 {
        /// Élément de rang `rank`, qui doit être inférieur à 47.
        pub const fn new(rank: u8) -> Self {
            Self(rank)
        }
    }

    impl Field for Gf47 {
        const ORDER: usize = ORDER;
        const GENERATOR: Self = Self(GENERATOR);

        fn zero() -> Self {
            Self(0)
        }

        fn one() -> Self {
            Self(1)
        }

        fn is_zero(self) -> bool {
            self.0 == 0
        }

        fn add(self, other: Self) -> Self {
            debug_assert!(usize::from(self.0) < ORDER && usize::from(other.0) < ORDER);
            Self(TABLES.0[usize::from(self.0) * ORDER + usize::from(other.0)])
        }

        fn sub(self, other: Self) -> Self {
            debug_assert!(usize::from(self.0) < ORDER && usize::from(other.0) < ORDER);
            let (a, b) = (usize::from(self.0), usize::from(other.0));
            Self(if a >= b {
                (a - b) as u8
            } else {
                (a + ORDER - b) as u8
            })
        }

        fn mul(self, other: Self) -> Self {
            debug_assert!(usize::from(self.0) < ORDER && usize::from(other.0) < ORDER);
            Self(TABLES.1[usize::from(self.0) * ORDER + usize::from(other.0)])
        }

        fn inv(self) -> Self {
            debug_assert!(usize::from(self.0) < ORDER);
            Self(TABLES.2[usize::from(self.0)])
        }
    }
}

/// GF(47²) : les `a + b·x` avec `x² = 5`, `a` et `b` dans GF(47). 5 n'est pas un carré modulo
/// 47 (c'est une racine primitive), donc x² − 5 est irréductible, et le corps compte 47² = 2 209
/// éléments.
pub mod gf2209 {
    use super::gf47::{Gf47, ORDER as BASE, TABLES};
    use super::Field;

    /// Nombre d'éléments : 47².
    pub const ORDER: usize = BASE * BASE;

    /// Un élément : `a + b·x`.
    #[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
    pub struct Gf2209 {
        pub a: u8,
        pub b: u8,
    }

    impl Gf2209 {
        const fn add(x: u8, y: u8) -> u8 {
            TABLES.0[(x as usize) * BASE + (y as usize)]
        }

        const fn mul(x: u8, y: u8) -> u8 {
            TABLES.1[(x as usize) * BASE + (y as usize)]
        }

        const fn neg(x: u8) -> u8 {
            if x == 0 {
                0
            } else {
                (BASE - x as usize) as u8
            }
        }

        /// Produit dans GF(47²) : (a₀ + a₁x)(b₀ + b₁x) = (a₀b₀ + 5·a₁b₁) + (a₀b₁ + a₁b₀)·x.
        const fn product(a: (u8, u8), b: (u8, u8)) -> (u8, u8) {
            (
                Self::add(Self::mul(a.0, b.0), Self::mul(5, Self::mul(a.1, b.1))),
                Self::add(Self::mul(a.0, b.1), Self::mul(a.1, b.0)),
            )
        }

        const fn power_is_one(mut base: (u8, u8), mut exponent: usize) -> bool {
            let mut result = (1u8, 0u8);
            while exponent > 0 {
                if exponent & 1 == 1 {
                    result = Self::product(result, base);
                }
                base = Self::product(base, base);
                exponent >>= 1;
            }
            result.0 == 1 && result.1 == 0
        }

        /// Un générateur du groupe multiplicatif (ordre 2 208 = 2⁵ · 3 · 23) : le premier
        /// `x + c`, par `c` croissant, dont les puissances 1 104, 736 et 96 ne valent pas 1.
        /// La recherche est faite à la compilation ; sa valeur fait partie du format.
        const fn find_generator() -> Self {
            let mut c = 1u8;
            loop {
                let candidate = (c, 1u8);
                if !Self::power_is_one(candidate, ORDER / 2)
                    && !Self::power_is_one(candidate, ORDER / 3)
                    && !Self::power_is_one(candidate, ORDER / 23)
                {
                    return Self { a: c, b: 1 };
                }
                c += 1;
            }
        }

        const GENERATOR: Self = Self::find_generator();

        /// L'élément de GF(47) de rang `rank` (inférieur à 47).
        pub const fn base(rank: u8) -> Self {
            Self { a: rank, b: 0 }
        }

        /// Vrai si l'élément appartient à GF(47).
        pub const fn is_base(self) -> bool {
            self.b == 0
        }

        /// Le rang dans GF(47), pour un élément qui lui appartient.
        pub const fn rank(self) -> u8 {
            self.a
        }
    }

    impl Field for Gf2209 {
        const ORDER: usize = ORDER;
        const GENERATOR: Self = Self::GENERATOR;

        fn zero() -> Self {
            Self { a: 0, b: 0 }
        }

        fn one() -> Self {
            Self { a: 1, b: 0 }
        }

        fn is_zero(self) -> bool {
            self.a == 0 && self.b == 0
        }

        fn add(self, other: Self) -> Self {
            Self {
                a: Self::add(self.a, other.a),
                b: Self::add(self.b, other.b),
            }
        }

        fn sub(self, other: Self) -> Self {
            self.add(Self {
                a: Self::neg(other.a),
                b: Self::neg(other.b),
            })
        }

        fn mul(self, other: Self) -> Self {
            let product = Self::product((self.a, self.b), (other.a, other.b));
            Self {
                a: product.0,
                b: product.1,
            }
        }

        fn inv(self) -> Self {
            // (a + bx)(a − bx) = a² − 5b² : la norme, un élément non nul de GF(47).
            debug_assert!(!self.is_zero());
            let five = Gf47(5);
            let norm = Gf47(self.a)
                .mul(Gf47(self.a))
                .sub(five.mul(Gf47(self.b)).mul(Gf47(self.b)));
            Self {
                a: norm.inv().0,
                b: 0,
            }
            .mul(Self {
                a: self.a,
                b: Self::neg(self.b),
            })
        }
    }
}
/// Polynôme à coefficients dans un corps, degré le plus faible en tête.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Poly<F: Field>(Vec<F>);

impl<F: Field> Poly<F> {
    fn zero() -> Self {
        Self(Vec::new())
    }

    fn constant(value: F) -> Self {
        if value.is_zero() {
            Self::zero()
        } else {
            Self(vec![value])
        }
    }

    fn is_zero(&self) -> bool {
        self.0.is_empty()
    }

    /// Degré ; `None` pour le polynôme nul.
    fn degree(&self) -> Option<usize> {
        self.0.len().checked_sub(1)
    }

    fn trim(&mut self) {
        while self
            .0
            .last()
            .is_some_and(|coefficient| coefficient.is_zero())
        {
            self.0.pop();
        }
    }

    fn eval(&self, x: F) -> F {
        self.0
            .iter()
            .rev()
            .fold(F::zero(), |acc, &coefficient| acc.mul(x).add(coefficient))
    }

    fn add(&self, other: &Self) -> Self {
        let mut out = self.0.clone();
        out.resize(other.0.len().max(out.len()), F::zero());
        for (slot, &value) in out.iter_mut().zip(&other.0) {
            *slot = slot.add(value);
        }
        let mut out = Self(out);
        out.trim();
        out
    }

    fn sub(&self, other: &Self) -> Self {
        let mut out = self.0.clone();
        out.resize(other.0.len().max(out.len()), F::zero());
        for (slot, &value) in out.iter_mut().zip(&other.0) {
            *slot = slot.sub(value);
        }
        let mut out = Self(out);
        out.trim();
        out
    }

    fn scale(&self, factor: F) -> Self {
        if factor.is_zero() {
            return Self::zero();
        }
        Self(self.0.iter().map(|&c| c.mul(factor)).collect())
    }

    fn mul(&self, other: &Self) -> Self {
        if self.is_zero() || other.is_zero() {
            return Self::zero();
        }
        let mut out = vec![F::zero(); self.0.len() + other.0.len() - 1];
        for (i, &a) in self.0.iter().enumerate() {
            if a.is_zero() {
                continue;
            }
            for (j, &b) in other.0.iter().enumerate() {
                out[i + j] = out[i + j].add(a.mul(b));
            }
        }
        let mut out = Self(out);
        out.trim();
        out
    }

    /// Multiplie en place par `(x − root)`.
    fn times_x_minus(&mut self, root: F) {
        let len = self.0.len();
        self.0.push(F::zero());
        for position in (1..=len).rev() {
            self.0[position] = self.0[position - 1].sub(root.mul(self.0[position]));
        }
        self.0[0] = F::zero().sub(root.mul(self.0[0]));
    }

    /// Division euclidienne : `(quotient, reste)`. `None` si le diviseur est nul.
    fn divmod(&self, divisor: &Self) -> Option<(Self, Self)> {
        if divisor.is_zero() {
            return None;
        }
        if self.is_zero() {
            return Some((Self::zero(), Self::zero()));
        }
        let mut remainder = self.clone();
        let divisor_degree = divisor.degree()?;
        if remainder.degree()? < divisor_degree {
            return Some((Self::zero(), remainder));
        }
        let lead = divisor.0.last()?.inv();
        let mut quotient = vec![F::zero(); remainder.0.len() - divisor.0.len() + 1];
        for offset in (0..quotient.len()).rev() {
            let factor = remainder.0[offset + divisor_degree].mul(lead);
            quotient[offset] = factor;
            if factor.is_zero() {
                continue;
            }
            for (j, &coefficient) in divisor.0.iter().enumerate() {
                remainder.0[offset + j] = remainder.0[offset + j].sub(factor.mul(coefficient));
            }
        }
        remainder.trim();
        Some((Self(quotient), remainder))
    }
}

/// Dénominateurs de la base de Lagrange de `points` : l'inverse de
/// `Π_{m ≠ i} (x_i − x_m)`, pour chaque `i`.
fn lagrange_denominators<F: Field>(points: &[F]) -> Vec<F> {
    let mut denominators = Vec::with_capacity(points.len());
    for (i, &point) in points.iter().enumerate() {
        let mut product = F::one();
        for (m, &other) in points.iter().enumerate() {
            if m != i {
                product = product.mul(point.sub(other));
            }
        }
        denominators.push(product.inv());
    }
    denominators
}

/// Écrit dans `weights` les poids de Lagrange de `points` en `target` : le polynôme
/// d'interpolation vaut `Σ weights[i] · valeurs[i]` en `target`. Produits cumulés à gauche et à
/// droite, pour n'évaluer qu'une fois chaque facteur.
fn lagrange_weights<F: Field>(points: &[F], denominators: &[F], target: F, weights: &mut [F]) {
    let mut below = vec![F::one(); points.len() + 1];
    for i in 0..points.len() {
        below[i + 1] = below[i].mul(target.sub(points[i]));
    }
    let mut above = F::one();
    for i in (0..points.len()).rev() {
        weights[i] = below[i].mul(above).mul(denominators[i]);
        above = above.mul(target.sub(points[i]));
    }
}

/// Produit scalaire.
fn dot<F: Field>(coefficients: &[F], values: &[F]) -> F {
    coefficients
        .iter()
        .zip(values)
        .fold(F::zero(), |acc, (&c, &v)| acc.add(c.mul(v)))
}

/// Polynôme de degré inférieur au nombre de points, qui vaut `values[i]` en `points[i]`.
///
/// Interpolation incrémentale, en O(n²).
fn interpolate<F: Field>(points: &[F], values: &[F]) -> Poly<F> {
    let mut result = Poly::zero();
    let mut product = Poly::constant(F::one());
    for (&x, &y) in points.iter().zip(values) {
        let numerator = y.sub(result.eval(x));
        let denominator = product.eval(x);
        result = result.add(&product.scale(numerator.div(denominator)));
        product.times_x_minus(x);
    }
    result
}

fn uncorrectable() -> DnaError {
    DnaError::Correction("bloc Reed-Solomon irrécupérable".to_string())
}

fn invalid(what: &str) -> DnaError {
    DnaError::Correction(format!("bloc Reed-Solomon invalide : {what}"))
}

/// Code de Reed-Solomon sur un corps, de longueur `n` et de dimension `k`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RsCode<F: Field> {
    n: usize,
    k: usize,
    /// Points d'évaluation : α⁰, α¹, …
    points: Vec<F>,
    /// Coefficients de la parité : pour chaque position de parité, la combinaison linéaire
    /// des `k` données (base de Lagrange).
    parity: Vec<F>,
}

impl<F: Field> RsCode<F> {
    /// Crée un code de `n` symboles dont `k` de donnée (`1 <= k < n <= ORDER − 1`).
    pub fn new(n: usize, k: usize) -> Result<Self> {
        if k == 0 || k >= n || n >= F::ORDER {
            return Err(DnaError::InvalidConfig(format!(
                "code Reed-Solomon invalide: ({n}, {k}) sur un corps de {} éléments",
                F::ORDER
            )));
        }
        let mut points = Vec::with_capacity(n);
        let mut x = F::one();
        for _ in 0..n {
            points.push(x);
            x = x.mul(F::GENERATOR);
        }
        let denominators = lagrange_denominators(&points[..k]);
        let mut parity = vec![F::zero(); (n - k) * k];
        let mut row = vec![F::zero(); k];
        for (rank, &target) in points[k..].iter().enumerate() {
            lagrange_weights(&points[..k], &denominators, target, &mut row);
            parity[rank * k..][..k].copy_from_slice(&row);
        }
        Ok(Self {
            n,
            k,
            points,
            parity,
        })
    }

    /// Longueur d'un bloc codé.
    pub fn block_len(&self) -> usize {
        self.n
    }

    /// Nombre de symboles de donnée.
    pub fn data_len(&self) -> usize {
        self.k
    }

    /// Nombre de symboles de parité, aussi le nombre d'effacements seuls que le code rétablit.
    pub fn parity_len(&self) -> usize {
        self.n - self.k
    }

    /// Nombre d'erreurs de position inconnue que le code corrige.
    pub fn correctable_errors(&self) -> usize {
        self.parity_len() / 2
    }

    /// Retourne `données || parité`.
    pub fn encode(&self, data: &[F]) -> Result<Vec<F>> {
        if data.len() != self.k {
            return Err(invalid("nombre de symboles de donnée inattendu"));
        }
        let mut block = Vec::with_capacity(self.n);
        block.extend_from_slice(data);
        block.resize(self.n, F::zero());
        self.add_parity(&mut block)?;
        Ok(block)
    }

    /// Écrit la parité d'un bloc dont les `k` premiers symboles sont les données.
    pub fn add_parity(&self, block: &mut [F]) -> Result<()> {
        if block.len() != self.n {
            return Err(invalid("bloc de longueur inattendue"));
        }
        let (data, parity) = block.split_at_mut(self.k);
        for (row, slot) in parity.iter_mut().enumerate() {
            *slot = dot(&self.parity[row * self.k..], data);
        }
        Ok(())
    }

    /// Vrai si le bloc est un mot de code.
    pub fn check(&self, block: &[F]) -> bool {
        if block.len() != self.n {
            return false;
        }
        let mut expected = block.to_vec();
        self.add_parity(&mut expected).is_ok() && expected == block
    }

    /// Corrige `block` en place, en s'appuyant sur les effacements de positions connues.
    ///
    /// Un bloc sain rend 0. Rend le nombre de symboles rétablis, effacements compris ; une
    /// erreur s'il est irrécupérable.
    pub fn correct(&self, block: &mut [F], erasures: &[usize]) -> Result<usize> {
        if block.len() != self.n {
            return Err(invalid("bloc de longueur inattendue"));
        }
        if erasures.len() > self.parity_len() {
            return Err(DnaError::Correction(format!(
                "{} effacements pour {} symboles de parité",
                erasures.len(),
                self.parity_len()
            )));
        }
        let mut erased = vec![false; self.n];
        for &position in erasures {
            if position >= self.n {
                return Err(invalid("effacement hors bloc"));
            }
            erased[position] = true;
        }
        if erasures.is_empty() && self.check(block) {
            return Ok(0);
        }

        // 1. Le polynôme produit des points conservés, et celui qui passe par les valeurs reçues.
        let kept: Vec<usize> = (0..self.n).filter(|&i| !erased[i]).collect();
        let mut g0 = Poly::constant(F::one());
        for &i in &kept {
            g0.times_x_minus(self.points[i]);
        }
        let points: Vec<F> = kept.iter().map(|&i| self.points[i]).collect();
        let values: Vec<F> = kept.iter().map(|&i| block[i]).collect();
        let g1 = interpolate(&points, &values);

        // 2. Euclide étendu : le premier reste de degré inférieur à (n' + k) / 2 est le
        //    produit du localisateur par le message.
        let limit = (kept.len() + self.k) / 2;
        let mut previous = g0;
        let mut current = g1;
        let mut previous_v = Poly::zero();
        let mut current_v = Poly::constant(F::one());
        while current.degree().is_some_and(|degree| degree >= limit) {
            let Some((quotient, remainder)) = previous.divmod(&current) else {
                return Err(uncorrectable());
            };
            let next_v = previous_v.sub(&quotient.mul(&current_v));
            previous = current;
            current = remainder;
            previous_v = current_v;
            current_v = next_v;
        }

        // 3. Message = reste / cofacteur, de degré inférieur à k. Le polynôme nul est un message
        //    comme un autre : le contrôle qui suit écarterait un reste nul qui ne vient pas de lui.
        let (message, rest) = current.divmod(&current_v).ok_or_else(uncorrectable)?;
        if !rest.is_zero() || message.degree().is_some_and(|degree| degree >= self.k) {
            return Err(uncorrectable());
        }

        // 4. Contrôle : le mot de code retrouvé ne doit s'écarter des valeurs reçues qu'autant
        //    que la capacité du code corrige d'erreurs.
        let recovered: Vec<F> = self.points.iter().map(|&x| message.eval(x)).collect();
        let changed = kept.iter().filter(|&&i| recovered[i] != block[i]).count();
        if changed > (kept.len() - self.k) / 2 {
            return Err(uncorrectable());
        }
        let fixed = block
            .iter()
            .zip(&recovered)
            .filter(|(&received, &value)| received != value)
            .count();
        block.copy_from_slice(&recovered);
        Ok(fixed)
    }

    /// Prépare le rétablissement d'un jeu d'effacements, partagé par plusieurs colonnes.
    ///
    /// Le coût de l'interpolation est payé une fois ; chaque colonne ne coûte plus que le
    /// produit de ses valeurs par les coefficients.
    pub fn solver(&self, erasures: &[usize]) -> Result<ErasureSolver<F>> {
        if erasures.len() > self.parity_len() {
            return Err(DnaError::Correction(format!(
                "{} effacements pour {} symboles de parité",
                erasures.len(),
                self.parity_len()
            )));
        }
        let mut erased = vec![false; self.n];
        for &position in erasures {
            if position >= self.n {
                return Err(invalid("effacement hors bloc"));
            }
            erased[position] = true;
        }
        // Base de résolution : les k premières positions conservées.
        let basis: Vec<usize> = (0..self.n).filter(|&i| !erased[i]).take(self.k).collect();

        // Dénominateurs de Lagrange de la base, puis, pour toute autre position, ses
        // coefficients sur la base. Produits cumulés à gauche et à droite, pour n'évaluer
        // qu'une fois chaque facteur.
        let basis_points: Vec<F> = basis.iter().map(|&i| self.points[i]).collect();
        let denominators = lagrange_denominators(&basis_points);
        let mut in_basis = vec![false; self.n];
        for &position in &basis {
            in_basis[position] = true;
        }
        let mut rows = Vec::with_capacity(self.n - self.k);
        let mut coefficients = vec![F::zero(); self.k];
        for position in 0..self.n {
            if in_basis[position] {
                continue;
            }
            lagrange_weights(
                &basis_points,
                &denominators,
                self.points[position],
                &mut coefficients,
            );
            rows.push(Row {
                position,
                erased: erased[position],
                coefficients: coefficients.clone(),
            });
        }
        Ok(ErasureSolver {
            basis,
            rows,
            filled: erasures.len(),
        })
    }
}

/// Une position hors de la base de résolution : conservée (à contrôler) ou effacée (à rétablir).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Row<F: Field> {
    position: usize,
    erased: bool,
    coefficients: Vec<F>,
}

/// Rétablit un jeu d'effacements, colonne après colonne.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErasureSolver<F: Field> {
    basis: Vec<usize>,
    rows: Vec<Row<F>>,
    filled: usize,
}

impl<F: Field> ErasureSolver<F> {
    /// Rétablit les symboles effacés d'un bloc, en place. Rend leur nombre.
    ///
    /// Une valeur conservée qui ne s'accorde pas avec la base rend une erreur : il y a une
    /// erreur quelque part, et le décodage complet ([`RsCode::correct`]) prend le relais.
    pub fn restore(&self, block: &mut [F]) -> Result<usize> {
        if block.len() != self.basis.len() + self.rows.len() {
            return Err(invalid("bloc de longueur inattendue"));
        }
        let values: Vec<F> = self.basis.iter().map(|&position| block[position]).collect();
        let mut restored = Vec::with_capacity(self.filled);
        for row in &self.rows {
            let value = dot(&row.coefficients, &values);
            if row.erased {
                restored.push((row.position, value));
            } else if block[row.position] != value {
                return Err(uncorrectable());
            }
        }
        for (position, value) in restored {
            block[position] = value;
        }
        Ok(self.filled)
    }
}

/// Code de Reed-Solomon sur GF(47) : le code interne d'un brin grass.
pub type Rs47Code = RsCode<gf47::Gf47>;
/// Code de Reed-Solomon sur GF(47²) : le code externe d'un bloc de brins grass.
pub type Rs2209Code = RsCode<gf2209::Gf2209>;

#[cfg(test)]
mod tests {
    use super::gf2209::Gf2209;
    use super::gf47::Gf47;
    use super::*;
    use crate::codec::rng::SeededRng;

    fn random_elements<F: Field + FromDraw>(rng: &mut SeededRng, len: usize) -> Vec<F> {
        (0..len)
            .map(|_| F::from_draw(rng.below(F::ORDER as u64)))
            .collect()
    }

    /// Construit un élément d'après un tirage uniforme de `[0, ORDER)`.
    trait FromDraw: Sized {
        fn from_draw(draw: u64) -> Self;
    }

    impl FromDraw for Gf47 {
        fn from_draw(draw: u64) -> Self {
            Self::new(draw as u8)
        }
    }

    impl FromDraw for Gf2209 {
        fn from_draw(draw: u64) -> Self {
            Self {
                a: (draw % 47) as u8,
                b: (draw / 47) as u8,
            }
        }
    }

    fn all_codes_47() -> Vec<(usize, usize)> {
        vec![
            (7, 3),
            (9, 7),
            (12, 11),
            (23, 19),
            (23, 15),
            (39, 25),
            (39, 35),
            (39, 38),
            (46, 40),
        ]
    }

    #[test]
    fn gf47_is_the_field_of_integers_modulo_47() {
        let mut seen = [false; 47];
        let mut x = Gf47::new(1);
        for _ in 1..47 {
            assert_eq!(x.mul(x.inv()), Gf47::new(1));
            assert_eq!(x.add(Gf47::new(13)).sub(Gf47::new(13)), x);
            seen[usize::from(x.0)] = true;
            x = x.mul(Gf47::GENERATOR);
        }
        assert!(seen[1..].iter().all(|&flag| flag), "5 est racine primitive");
        assert_eq!(x, Gf47::new(1));
        assert_eq!(Gf47::new(46).add(Gf47::new(1)), Gf47::new(0));
        assert_eq!(Gf47::new(3).sub(Gf47::new(5)), Gf47::new(45));
        assert_eq!(Gf47::new(46).mul(Gf47::new(46)), Gf47::new(1));
        assert_eq!(
            Gf47::new(1).div(Gf47::new(5)),
            Gf47::new(19),
            "5 × 19 = 95 = 2 × 47 + 1"
        );
        assert_eq!(Gf47::new(0).inv(), Gf47::new(0));
        assert_eq!(
            Gf47::new(3).pow(46),
            Gf47::new(1),
            "petit théorème de Fermat"
        );
        assert_eq!(Gf47::zero(), Gf47::new(0));
        assert_eq!(Gf47::one(), Gf47::new(1));
        assert!(Gf47::new(0).is_zero() && !Gf47::new(1).is_zero());
    }

    #[test]
    fn gf2209_is_a_field_of_2209_elements() {
        use gf2209::ORDER;
        assert_eq!(ORDER, 2209);
        // Tout élément non nul a un inverse ; le générateur les parcourt tous : 2 208 puissances,
        // de α⁰ à α²²⁰⁷.
        let mut seen = vec![false; ORDER];
        let mut x = Gf2209::one();
        for _ in 0..ORDER - 1 {
            assert_eq!(x.mul(x.inv()), Gf2209::one(), "{x:?}");
            assert!(!x.is_zero());
            seen[usize::from(x.a) + 47 * usize::from(x.b)] = true;
            x = x.mul(Gf2209::GENERATOR);
        }
        assert!(seen[1..].iter().all(|&flag| flag));
        assert_eq!(x, Gf2209::one());
        assert!(Gf2209::GENERATOR.pow(ORDER / 2) != Gf2209::one());
        assert!(Gf2209::GENERATOR.pow(ORDER / 3) != Gf2209::one());
        assert!(Gf2209::GENERATOR.pow(ORDER / 23) != Gf2209::one());
        // La forme du générateur est inscrite dans le format : x + 4.
        assert_eq!(Gf2209::GENERATOR, Gf2209 { a: 4, b: 1 });
        // Loi de corps, sur 2 000 triplets tirés au hasard.
        let mut rng = SeededRng::new(3, 71);
        for _ in 0..2_000 {
            let (a, b, c) = (
                random_elements::<Gf2209>(&mut rng, 1)[0],
                random_elements::<Gf2209>(&mut rng, 1)[0],
                random_elements::<Gf2209>(&mut rng, 1)[0],
            );
            assert_eq!(a.mul(b).mul(c), a.mul(b.mul(c)));
            assert_eq!(a.mul(b), b.mul(a));
            assert_eq!(a.add(b), b.add(a));
            assert_eq!(a.sub(b).add(b), a);
            if !a.is_zero() {
                assert_eq!(a.inv().inv(), a);
            }
            // GF(47) est un sous-corps : la distributivité y est exacte.
            let p = Gf2209::base(13);
            assert!(p.is_base());
            assert_eq!(p.rank(), 13);
            assert_eq!(p.mul(a.add(b)), p.mul(a).add(p.mul(b)));
            assert_eq!(p.mul(Gf2209::zero()), Gf2209::zero());
        }
        assert_eq!(Gf2209::base(5).pow(46), Gf2209::one());
        assert_eq!(Gf2209::GENERATOR.pow(2208), Gf2209::one());
        // La forme du générateur est inscrite dans le format.
        eprintln!("générateur de GF(47²) : {:?}", Gf2209::GENERATOR);
        assert_eq!(
            Gf2209 { a: 0, b: 1 }.mul(Gf2209 { a: 0, b: 1 }),
            Gf2209::base(5)
        );
    }

    #[test]
    fn every_code_roundtrips_and_checks() {
        for (n, k) in all_codes_47() {
            let code = Rs47Code::new(n, k).unwrap();
            assert_eq!(code.parity_len(), n - k);
            let mut rng = SeededRng::new(n as u64, 73);
            let data = random_elements::<Gf47>(&mut rng, k);
            let block = code.encode(&data).unwrap();
            assert_eq!(block.len(), n);
            assert_eq!(&block[..k], &data[..]);
            assert!(code.check(&block));
            let mut repaired = block.clone();
            assert_eq!(code.correct(&mut repaired, &[]).unwrap(), 0);
            assert_eq!(repaired, block);
        }
        for (n, k) in [
            (10, 6),
            (46, 40),
            (93, 1),
            (100, 95),
            (713, 621),
            (800, 700),
        ] {
            let code = Rs2209Code::new(n, k).unwrap();
            let mut rng = SeededRng::new(n as u64, 79);
            let data = random_elements::<Gf2209>(&mut rng, k);
            let block = code.encode(&data).unwrap();
            assert!(code.check(&block));
            let mut repaired = block.clone();
            assert_eq!(code.correct(&mut repaired, &[]).unwrap(), 0);
            assert_eq!(repaired, block);
        }
    }

    #[test]
    fn every_pattern_within_the_capacity_is_corrected_exhaustively() {
        // Code (7, 5) sur GF(47) : capacité 2. Toute erreur seule, tout effacement seul, tout
        // couple d'effacements ; deux erreurs, ou une erreur et un effacement, dépassent.
        let code = Rs47Code::new(7, 5).unwrap();
        let mut rng = SeededRng::new(1, 83);
        let data = random_elements::<Gf47>(&mut rng, 5);
        let block = code.encode(&data).unwrap();
        for position in 0..7 {
            for delta in 1..47u8 {
                let mut repaired = block.clone();
                repaired[position] = repaired[position].add(Gf47::new(delta));
                code.correct(&mut repaired, &[]).unwrap_or_else(|error| {
                    panic!("erreur en {position}, écart {delta} : {error}")
                });
                assert_eq!(repaired, block, "erreur en {position}, écart {delta}");
                let mut erased = block.clone();
                erased[position] = Gf47::new(46);
                code.correct(&mut erased, &[position]).unwrap();
                assert_eq!(erased, block);
            }
        }
        for first in 0..7 {
            for second in first + 1..7 {
                let mut damaged = block.clone();
                damaged[first] = Gf47::new(46);
                damaged[first] = damaged[first].add(Gf47::new(1));
                damaged[second] = Gf47::new(46);
                code.correct(&mut damaged, &[first, second])
                    .unwrap_or_else(|error| panic!("effacements {first}/{second} : {error}"));
                assert_eq!(damaged, block);
            }
        }
        let mut damaged = block.clone();
        damaged[2] = damaged[2].add(Gf47::new(5));
        damaged[4] = damaged[4].add(Gf47::new(5));
        assert!(code.correct(&mut damaged, &[]).is_err());
        let mut damaged = block.clone();
        damaged[2] = Gf47::new(46);
        damaged[4] = damaged[4].add(Gf47::new(5));
        assert!(code.correct(&mut damaged, &[2]).is_err());
    }

    #[test]
    fn errors_and_erasures_up_to_the_capacity_are_corrected() {
        let mut rng = SeededRng::new(7, 89);
        for (n, k) in all_codes_47() {
            let code = Rs47Code::new(n, k).unwrap();
            check_patterns(&code, n, k, &mut rng);
        }
        for (n, k) in [(10, 6), (46, 40), (93, 1), (713, 621)] {
            let code = Rs2209Code::new(n, k).unwrap();
            check_patterns(&code, n, k, &mut rng);
        }
    }

    fn check_patterns<F: Field + FromDraw>(
        code: &RsCode<F>,
        n: usize,
        k: usize,
        rng: &mut SeededRng,
    ) {
        let capacity = n - k;
        for trial in 0..40u64 {
            let data = random_elements(rng, k);
            let block = code.encode(&data).unwrap();
            let erasure_count = (trial as usize) % (capacity + 1);
            let error_count = (capacity - erasure_count) / 2;
            let mut positions: Vec<usize> = (0..n).collect();
            for i in (1..n).rev() {
                positions.swap(i, rng.below(i as u64 + 1) as usize);
            }
            let mut damaged = block.clone();
            for &position in &positions[..erasure_count] {
                damaged[position] = block[position].add(F::one());
            }
            for &position in &positions[erasure_count..erasure_count + error_count] {
                damaged[position] = damaged[position].add(F::one());
            }
            let erasures = &positions[..erasure_count];
            let expected = erasure_count + error_count;
            let fixed = code
                .correct(&mut damaged, erasures)
                .unwrap_or_else(|error| panic!("({n}, {k}), tirage {trial} : {error}"));
            assert_eq!(fixed, expected, "({n}, {k}), tirage {trial}");
            assert_eq!(damaged, block, "({n}, {k}), tirage {trial}");
        }
    }

    #[test]
    fn beyond_the_capacity_no_silent_result_that_is_not_a_codeword() {
        let mut rng = SeededRng::new(11, 97);
        for (n, k) in [(7, 5), (23, 19), (39, 25)] {
            let code = Rs47Code::new(n, k).unwrap();
            let data = random_elements::<Gf47>(&mut rng, k);
            let block = code.encode(&data).unwrap();
            for _ in 0..300 {
                let mut positions: Vec<usize> = (0..n).collect();
                for i in (1..n).rev() {
                    positions.swap(i, rng.below(i as u64 + 1) as usize);
                }
                let mut damaged = block.clone();
                for &position in &positions[..n - k] {
                    damaged[position] = damaged[position].add(Gf47::new(1 + rng.below(46) as u8));
                }
                // Soit l'échec est annoncé, soit le résultat est un autre mot de code : c'est
                // alors l'empreinte SHA-256 de l'enveloppe qui l'intercepte.
                if code.correct(&mut damaged, &[]).is_ok() {
                    assert!(code.check(&damaged));
                }
            }
        }
    }

    #[test]
    fn inconsistent_inputs_are_refused() {
        assert!(Rs47Code::new(0, 0).is_err());
        assert!(Rs47Code::new(5, 0).is_err());
        assert!(Rs47Code::new(5, 5).is_err());
        assert!(Rs47Code::new(5, 6).is_err());
        assert!(Rs47Code::new(47, 40).is_err());
        assert!(Rs47Code::new(46, 40).is_ok());
        assert!(Rs2209Code::new(2209, 2200).is_err());
        assert!(Rs2209Code::new(2208, 2200).is_ok());
        let code = Rs47Code::new(7, 5).unwrap();
        assert!(code.encode(&[Gf47::new(1), Gf47::new(2)]).is_err());
        assert!(!code.check(&[Gf47::new(1), Gf47::new(2), Gf47::new(3)]));
        let mut block = vec![Gf47::new(0); 6];
        assert!(code.correct(&mut block, &[]).is_err());
        let mut block = code
            .encode(&[
                Gf47::new(1),
                Gf47::new(2),
                Gf47::new(3),
                Gf47::new(4),
                Gf47::new(5),
            ])
            .unwrap();
        assert!(code.correct(&mut block, &[7]).is_err());
        assert!(code.correct(&mut block, &[0, 1, 2]).is_err());
        // Deux erreurs et un effacement : 2 × 1 + 1 = 3 dépasse la capacité de 2.
        block[1] = block[1].add(Gf47::new(3));
        block[4] = block[4].add(Gf47::new(5));
        assert!(code.correct(&mut block, &[6]).is_err());
        assert!(code.solver(&[0, 1, 2]).is_err());
        assert!(code.solver(&[7]).is_err());
    }

    #[test]
    fn a_constant_block_is_decoded_even_with_erasures() {
        let code = Rs47Code::new(23, 19).unwrap();
        let mut block = vec![Gf47::new(9); 23];
        for position in [0usize, 5, 22, 13] {
            block[position] = Gf47::new(0);
        }
        assert_eq!(code.correct(&mut block, &[0, 5, 22, 13]).unwrap(), 4);
        assert_eq!(block, vec![Gf47::new(9); 23]);
    }

    #[test]
    fn the_zero_codeword_is_corrected_like_any_other() {
        // La première colonne d'un groupe du schéma grass est presque toujours nulle : des
        // effacements et des erreurs dessus doivent se réparer.
        for (n, k) in [(7usize, 3usize), (23, 19), (39, 25)] {
            let code = Rs47Code::new(n, k).unwrap();
            let capacity = n - k;
            for count in 1..=capacity {
                let mut damaged = vec![Gf47::new(0); n];
                let erasures: Vec<usize> = (0..count).collect();
                for &position in &erasures {
                    damaged[position] = Gf47::new(5);
                }
                assert_eq!(
                    code.correct(&mut damaged, &erasures).unwrap(),
                    count,
                    "({n}, {k}), {count} effacements"
                );
                assert_eq!(damaged, vec![Gf47::new(0); n]);
            }
            for count in 1..=capacity / 2 {
                let mut damaged = vec![Gf47::new(0); n];
                for slot in damaged.iter_mut().take(count) {
                    *slot = Gf47::new(46);
                }
                assert_eq!(code.correct(&mut damaged, &[]).unwrap(), count);
                assert_eq!(damaged, vec![Gf47::new(0); n]);
            }
            let mut damaged = vec![Gf47::new(0); n];
            for slot in damaged.iter_mut().take(capacity) {
                *slot = Gf47::new(46);
            }
            if code.correct(&mut damaged, &[]).is_ok() {
                assert!(code.check(&damaged));
            }
        }
        for (n, k) in [(93usize, 1usize), (713, 621)] {
            let code = Rs2209Code::new(n, k).unwrap();
            let mut damaged = vec![Gf2209::zero(); n];
            let erasures: Vec<usize> = (0..n - k).collect();
            for &position in &erasures {
                damaged[position] = Gf2209 { a: 3, b: 9 };
            }
            assert_eq!(code.correct(&mut damaged, &erasures).unwrap(), n - k);
            assert!(damaged.iter().all(|value| value.is_zero()));
        }
        let one = Poly(vec![Gf47::new(1), Gf47::new(2)]);
        assert_eq!(
            Poly::<Gf47>::zero().divmod(&one),
            Some((Poly::zero(), Poly::zero()))
        );
    }

    #[test]
    fn the_erasure_solver_restores_columns_and_detects_a_false_kept_value() {
        let mut rng = SeededRng::new(13, 101);
        for (n, k) in [(23usize, 19usize), (39, 35), (46, 40)] {
            check_solver(&Rs47Code::new(n, k).unwrap(), n, k, &mut rng);
        }
        for (n, k) in [(93usize, 1usize), (713, 621)] {
            check_solver(&Rs2209Code::new(n, k).unwrap(), n, k, &mut rng);
        }
    }

    fn check_solver<F: Field + FromDraw>(
        code: &RsCode<F>,
        n: usize,
        k: usize,
        rng: &mut SeededRng,
    ) {
        let parity = n - k;
        for trial in 0..12usize {
            let count = 1 + trial % parity;
            let mut positions: Vec<usize> = (0..n).collect();
            for i in (1..n).rev() {
                positions.swap(i, rng.below(i as u64 + 1) as usize);
            }
            positions[..count].sort_unstable();
            let erasures = &positions[..count];
            let data = random_elements(rng, k);
            let block = code.encode(&data).unwrap();
            let solver = code.solver(erasures).unwrap();

            // Plusieurs colonnes, mêmes effacements.
            let mut columns: Vec<Vec<F>> = Vec::new();
            for _ in 0..5 {
                let column = code.encode(&random_elements(rng, k)).unwrap();
                columns.push(column);
            }
            let intact: Vec<Vec<F>> = columns.clone();
            for column in &mut columns {
                for &position in erasures {
                    column[position] = F::zero();
                }
                assert_eq!(solver.restore(column).unwrap(), count);
            }
            assert_eq!(columns, intact, "({n}, {k}), tirage {trial}");
            assert!(code.check(&block));

            // Une valeur conservée fausse : annoncée dès qu'il reste une position conservée à
            // contrôler, et le décodage complet corrige (une erreur) si la parité le permet.
            // Parité entière utilisée par les effacements, il n'y a plus rien à contrôler.
            if erasures.len() < parity {
                let mut column = intact[0].clone();
                for &position in erasures {
                    column[position] = F::zero();
                }
                let kept: Vec<usize> = (0..n)
                    .filter(|position| !erasures.contains(position))
                    .collect();
                let wrong = kept[rng.below(kept.len() as u64) as usize];
                column[wrong] = column[wrong].add(F::one());
                assert!(
                    solver.restore(&mut column).is_err(),
                    "({n}, {k}), tirage {trial}"
                );
                if erasures.len() + 2 <= parity {
                    code.correct(&mut column, erasures).unwrap();
                    assert_eq!(column, intact[0]);
                }
            }
        }
    }

    #[test]
    fn polynomial_arithmetic_is_consistent() {
        let a = Poly(vec![Gf47::new(1), Gf47::new(2), Gf47::new(3)]);
        let b = Poly(vec![Gf47::new(5), Gf47::new(0), Gf47::new(1)]);
        let product = a.mul(&b);
        assert_eq!(
            product,
            Poly(vec![
                Gf47::new(5),
                Gf47::new(10),
                Gf47::new(16),
                Gf47::new(2),
                Gf47::new(3)
            ])
        );
        let (quotient, remainder) = product.divmod(&a).unwrap();
        assert_eq!((quotient, remainder), (b.clone(), Poly::zero()));
        assert_eq!(product.divmod(&Poly::zero()), None);
        let (quotient, remainder) = a.divmod(&product).unwrap();
        assert_eq!(quotient, Poly::zero());
        assert_eq!(remainder, a);
        assert_eq!(a.sub(&a), Poly::zero());
        assert_eq!(
            a.add(&a),
            Poly(vec![Gf47::new(2), Gf47::new(4), Gf47::new(6)])
        );
        let mut linear = Poly(vec![Gf47::new(3), Gf47::new(1)]);
        linear.times_x_minus(Gf47::new(2));
        // (3 + x)(x − 2) = x² + x − 6, et −6 vaut 41 modulo 47.
        assert_eq!(
            linear,
            Poly(vec![Gf47::new(41), Gf47::new(1), Gf47::new(1)])
        );
        assert_eq!(a.eval(Gf47::new(0)), Gf47::new(1));
        assert_eq!(a.eval(Gf47::new(1)), Gf47::new(6));
        assert_eq!(a.eval(Gf47::new(2)), Gf47::new(17));
        let points = [Gf47::new(1), Gf47::new(5), Gf47::new(25)];
        let polynomial = interpolate(&points, &[Gf47::new(3), Gf47::new(4), Gf47::new(9)]);
        for (&x, y) in points
            .iter()
            .zip([Gf47::new(3), Gf47::new(4), Gf47::new(9)])
        {
            assert_eq!(polynomial.eval(x), y);
        }
    }
}
