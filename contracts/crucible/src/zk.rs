//! Mock host functions for BN254 / BLS12-381 pairing arithmetic and Groth16 / Plonk
//! verification.
//!
//! **Host-only:** These types stand in for Soroban pairing-curve host functions so
//! privacy-preserving and zk-rollup settlement contracts can be unit-tested without
//! a full cryptographic backend. Arithmetic is bilinear over a wrapping `u64`
//! field — it is *not* the real BN254 or BLS12-381 group law.

use soroban_sdk::{Bytes, Env};

/// Pairing-friendly curves exposed by the mock harness.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum PairingCurve {
    /// BN254 (alt_bn128) — Ethereum-compatible Groth16.
    Bn254,
    /// BLS12-381 — used by many Plonk / Groth16 circuits.
    Bls12_381,
}

impl PairingCurve {
    /// Discriminator written into serialized points so proofs cannot be mixed
    /// across curves.
    pub fn tag(self) -> u8 {
        match self {
            PairingCurve::Bn254 => 0xB1,
            PairingCurve::Bls12_381 => 0xB2,
        }
    }
}

/// Affine G1 point (mock).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct G1 {
    pub x: u64,
    pub y: u64,
    pub inf: bool,
}

impl G1 {
    pub fn identity() -> Self {
        Self {
            x: 0,
            y: 0,
            inf: true,
        }
    }

    pub fn generator() -> Self {
        Self {
            x: 1,
            y: 2,
            inf: false,
        }
    }

    pub fn is_identity(&self) -> bool {
        self.inf
    }

    /// 64-byte compressed mock encoding: `[x le | y le | tag | zeros…]`.
    pub fn to_bytes(&self, env: &Env, curve: PairingCurve) -> Bytes {
        let mut raw = [0u8; 64];
        raw[0..8].copy_from_slice(&self.x.to_le_bytes());
        raw[8..16].copy_from_slice(&self.y.to_le_bytes());
        raw[16] = curve.tag();
        raw[17] = if self.inf { 1 } else { 0 };
        let mut bytes = Bytes::new(env);
        bytes.extend_from_array(&raw);
        bytes
    }

    pub fn from_bytes(bytes: &Bytes) -> Self {
        if bytes.len() < 18 {
            return Self::identity();
        }
        Self {
            x: read_u64_le(bytes, 0),
            y: read_u64_le(bytes, 8),
            inf: bytes.get(17).unwrap_or(0) != 0,
        }
    }
}

/// Affine G2 point over a mock Fp2 (two `u64` coordinates).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct G2 {
    pub x: [u64; 2],
    pub y: [u64; 2],
    pub inf: bool,
}

impl G2 {
    pub fn identity() -> Self {
        Self {
            x: [0, 0],
            y: [0, 0],
            inf: true,
        }
    }

    pub fn generator() -> Self {
        Self {
            x: [1, 0],
            y: [0, 1],
            inf: false,
        }
    }

    pub fn to_bytes(&self, env: &Env, curve: PairingCurve) -> Bytes {
        let mut raw = [0u8; 128];
        raw[0..8].copy_from_slice(&self.x[0].to_le_bytes());
        raw[8..16].copy_from_slice(&self.x[1].to_le_bytes());
        raw[16..24].copy_from_slice(&self.y[0].to_le_bytes());
        raw[24..32].copy_from_slice(&self.y[1].to_le_bytes());
        raw[32] = curve.tag();
        raw[33] = if self.inf { 1 } else { 0 };
        let mut bytes = Bytes::new(env);
        bytes.extend_from_array(&raw);
        bytes
    }

    pub fn from_bytes(bytes: &Bytes) -> Self {
        if bytes.len() < 34 {
            return Self::identity();
        }
        Self {
            x: [read_u64_le(bytes, 0), read_u64_le(bytes, 8)],
            y: [read_u64_le(bytes, 16), read_u64_le(bytes, 24)],
            inf: bytes.get(33).unwrap_or(0) != 0,
        }
    }
}

/// Target group element. Mock pairing is encoded additively: `1` is `0`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct Gt {
    pub exp: u64,
}

impl Gt {
    pub fn one() -> Self {
        Self { exp: 0 }
    }

    pub fn mul(self, other: Self) -> Self {
        Self {
            exp: self.exp.wrapping_add(other.exp),
        }
    }

    pub fn inverse(self) -> Self {
        Self {
            exp: self.exp.wrapping_neg(),
        }
    }

    pub fn is_one(self) -> bool {
        self.exp == 0
    }
}

/// Groth16 proving / verification key (mock).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Groth16VerifyingKey {
    pub curve: PairingCurve,
    pub alpha_g1: G1,
    pub beta_g2: G2,
    pub gamma_g2: G2,
    pub delta_g2: G2,
    pub ic: Vec<G1>,
}

/// Groth16 proof (A ∈ G1, B ∈ G2, C ∈ G1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Groth16Proof {
    pub curve: PairingCurve,
    pub a: G1,
    pub b: G2,
    pub c: G1,
}

/// Simplified Plonk proof: two G1 openings and a G2 quotient commitment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlonkProof {
    pub curve: PairingCurve,
    pub a: G1,
    pub b: G1,
    pub c: G2,
}

pub fn g1_add(p: G1, q: G1) -> G1 {
    if p.inf {
        return q;
    }
    if q.inf {
        return p;
    }
    G1 {
        x: p.x.wrapping_add(q.x),
        y: p.y.wrapping_add(q.y),
        inf: false,
    }
}

pub fn g1_mul(p: G1, scalar: u64) -> G1 {
    if p.inf || scalar == 0 {
        return G1::identity();
    }
    G1 {
        x: p.x.wrapping_mul(scalar),
        y: p.y.wrapping_mul(scalar),
        inf: false,
    }
}

pub fn g1_neg(p: G1) -> G1 {
    if p.inf {
        return p;
    }
    G1 {
        x: p.x.wrapping_neg(),
        y: p.y.wrapping_neg(),
        inf: false,
    }
}

pub fn g2_add(p: G2, q: G2) -> G2 {
    if p.inf {
        return q;
    }
    if q.inf {
        return p;
    }
    G2 {
        x: [p.x[0].wrapping_add(q.x[0]), p.x[1].wrapping_add(q.x[1])],
        y: [p.y[0].wrapping_add(q.y[0]), p.y[1].wrapping_add(q.y[1])],
        inf: false,
    }
}

pub fn g2_mul(p: G2, scalar: u64) -> G2 {
    if p.inf || scalar == 0 {
        return G2::identity();
    }
    G2 {
        x: [p.x[0].wrapping_mul(scalar), p.x[1].wrapping_mul(scalar)],
        y: [p.y[0].wrapping_mul(scalar), p.y[1].wrapping_mul(scalar)],
        inf: false,
    }
}

pub fn g2_neg(p: G2) -> G2 {
    if p.inf {
        return p;
    }
    G2 {
        x: [p.x[0].wrapping_neg(), p.x[1].wrapping_neg()],
        y: [p.y[0].wrapping_neg(), p.y[1].wrapping_neg()],
        inf: false,
    }
}

/// Bilinear mock pairing `e: G1 × G2 → GT`.
///
/// `e(sP, Q) = e(P, Q)^s` and `e(P, tQ) = e(P, Q)^t` hold under wrapping `u64`
/// arithmetic, which is enough to exercise Groth16 / Plonk pairing checks.
pub fn pairing(p: G1, q: G2) -> Gt {
    if p.inf || q.inf {
        return Gt::one();
    }
    Gt {
        exp: p.x.wrapping_mul(q.x[0]),
    }
}

/// Product of pairings equals `1` iff the wrapped exponents sum to zero.
pub fn pairing_check(pairs: &[(G1, G2)]) -> bool {
    let mut acc = Gt::one();
    for (p, q) in pairs {
        acc = acc.mul(pairing(*p, *q));
    }
    acc.is_one()
}

/// Linear combination `IC[0] + Σ input_i · IC[i+1]`.
pub fn groth16_linear_combination(vk: &Groth16VerifyingKey, public_inputs: &[u64]) -> Option<G1> {
    if vk.ic.is_empty() || vk.ic.len() != public_inputs.len() + 1 {
        return None;
    }
    let mut acc = vk.ic[0];
    for (input, ic) in public_inputs.iter().zip(vk.ic.iter().skip(1)) {
        acc = g1_add(acc, g1_mul(*ic, *input));
    }
    Some(acc)
}

/// Groth16 verification: `e(A,B) = e(α,β) · e(L,γ) · e(C,δ)`.
pub fn verify_groth16(
    vk: &Groth16VerifyingKey,
    proof: &Groth16Proof,
    public_inputs: &[u64],
) -> bool {
    if proof.curve != vk.curve {
        return false;
    }
    if proof.a.inf || proof.b.inf || proof.c.inf {
        return false;
    }
    let l = match groth16_linear_combination(vk, public_inputs) {
        Some(p) => p,
        None => return false,
    };
    pairing_check(&[
        (proof.a, proof.b),
        (g1_neg(vk.alpha_g1), vk.beta_g2),
        (g1_neg(l), vk.gamma_g2),
        (g1_neg(proof.c), vk.delta_g2),
    ])
}

/// Produce a proof that satisfies the Groth16 pairing equation for `vk`.
pub fn generate_groth16_proof(
    vk: &Groth16VerifyingKey,
    public_inputs: &[u64],
) -> Option<Groth16Proof> {
    let l = groth16_linear_combination(vk, public_inputs)?;
    // A = G1 generator so B can be solved without a field inverse.
    let a = G1::generator();
    let c = g1_mul(G1::generator(), 7);
    let rhs = pairing(vk.alpha_g1, vk.beta_g2)
        .mul(pairing(l, vk.gamma_g2))
        .mul(pairing(c, vk.delta_g2));
    // e(A, B) = A.x * B.x[0] = rhs.exp, A.x = 1 ⇒ B.x[0] = rhs.exp
    let b = G2 {
        x: [rhs.exp, 0],
        y: [0, 1],
        inf: false,
    };
    Some(Groth16Proof {
        curve: vk.curve,
        a,
        b,
        c,
    })
}

/// Tamper with `A` so the pairing equation fails (negative test fixture).
pub fn tamper_groth16_proof(proof: Groth16Proof) -> Groth16Proof {
    let mut tampered = proof;
    tampered.a.x = proof.a.x.wrapping_add(1);
    tampered
}

/// Plonk mock: `e(A + pub·G, C) = e(B, H)` where `H` is the G2 generator.
pub fn verify_plonk(curve: PairingCurve, proof: &PlonkProof, public_input: u64) -> bool {
    if proof.curve != curve {
        return false;
    }
    let shifted = g1_add(proof.a, g1_mul(G1::generator(), public_input));
    pairing_check(&[(shifted, proof.c), (g1_neg(proof.b), G2::generator())])
}

pub fn generate_plonk_proof(curve: PairingCurve, public_input: u64) -> PlonkProof {
    let a = g1_mul(G1::generator(), 3);
    let shifted = g1_add(a, g1_mul(G1::generator(), public_input));
    let c = G2::generator();
    // e(shifted, C) = shifted.x * 1; need e(B, G2) = B.x * 1 equal to that.
    let b = G1 {
        x: shifted.x,
        y: shifted.y,
        inf: false,
    };
    PlonkProof { curve, a, b, c }
}

pub fn tamper_plonk_proof(proof: PlonkProof) -> PlonkProof {
    let mut tampered = proof;
    tampered.a.x = proof.a.x.wrapping_add(1);
    tampered
}

/// Deterministic mock verifying key for tests.
pub fn sample_verifying_key(curve: PairingCurve, n_public: usize) -> Groth16VerifyingKey {
    let mut ic = Vec::with_capacity(n_public + 1);
    ic.push(g1_mul(G1::generator(), 11));
    for i in 0..n_public {
        ic.push(g1_mul(G1::generator(), 13 + i as u64));
    }
    Groth16VerifyingKey {
        curve,
        alpha_g1: g1_mul(G1::generator(), 3),
        beta_g2: g2_mul(G2::generator(), 5),
        gamma_g2: g2_mul(G2::generator(), 17),
        delta_g2: g2_mul(G2::generator(), 19),
        ic,
    }
}

pub fn encode_scalar(env: &Env, scalar: u64) -> Bytes {
    let mut raw = [0u8; 32];
    raw[0..8].copy_from_slice(&scalar.to_le_bytes());
    let mut bytes = Bytes::new(env);
    bytes.extend_from_array(&raw);
    bytes
}

pub fn decode_scalar(bytes: &Bytes) -> u64 {
    read_u64_le(bytes, 0)
}

pub fn curve_tag_from_g1_bytes(bytes: &Bytes) -> Option<u8> {
    bytes.get(16)
}

fn read_u64_le(bytes: &Bytes, offset: u32) -> u64 {
    let mut out = [0u8; 8];
    for i in 0..8u32 {
        out[i as usize] = bytes.get(offset + i).unwrap_or(0);
    }
    u64::from_le_bytes(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_bilinear(curve: PairingCurve) {
        let _ = curve;
        let p = G1::generator();
        let q = G2::generator();
        let s = 9u64;
        let t = 4u64;
        assert_eq!(pairing(g1_mul(p, s), q), pairing(p, g2_mul(q, s)));
        assert_eq!(
            pairing(g1_add(g1_mul(p, s), g1_mul(p, t)), q),
            pairing(g1_mul(p, s), q).mul(pairing(g1_mul(p, t), q))
        );
    }

    #[test]
    fn bn254_and_bls12_381_pairings_are_bilinear() {
        assert_bilinear(PairingCurve::Bn254);
        assert_bilinear(PairingCurve::Bls12_381);
    }

    #[test]
    fn g1_identity_is_pairing_identity() {
        assert!(pairing(G1::identity(), G2::generator()).is_one());
        assert!(pairing(G1::generator(), G2::identity()).is_one());
    }

    #[test]
    fn groth16_valid_proof_verifies_on_both_curves() {
        for curve in [PairingCurve::Bn254, PairingCurve::Bls12_381] {
            let vk = sample_verifying_key(curve, 2);
            let inputs = [4u64, 8];
            let proof = generate_groth16_proof(&vk, &inputs).unwrap();
            assert!(verify_groth16(&vk, &proof, &inputs));
        }
    }

    #[test]
    fn groth16_tampered_proof_is_rejected() {
        let vk = sample_verifying_key(PairingCurve::Bn254, 1);
        let inputs = [6u64];
        let proof = generate_groth16_proof(&vk, &inputs).unwrap();
        let bad = tamper_groth16_proof(proof);
        assert!(!verify_groth16(&vk, &bad, &inputs));
    }

    #[test]
    fn groth16_wrong_public_inputs_fail() {
        let vk = sample_verifying_key(PairingCurve::Bls12_381, 1);
        let proof = generate_groth16_proof(&vk, &[3]).unwrap();
        assert!(!verify_groth16(&vk, &proof, &[99]));
    }

    #[test]
    fn groth16_rejects_cross_curve_proof() {
        let vk = sample_verifying_key(PairingCurve::Bn254, 1);
        let mut proof = generate_groth16_proof(&vk, &[1]).unwrap();
        proof.curve = PairingCurve::Bls12_381;
        assert!(!verify_groth16(&vk, &proof, &[1]));
    }

    #[test]
    fn plonk_valid_and_negative_cases() {
        let proof = generate_plonk_proof(PairingCurve::Bls12_381, 42);
        assert!(verify_plonk(PairingCurve::Bls12_381, &proof, 42));
        assert!(!verify_plonk(
            PairingCurve::Bls12_381,
            &tamper_plonk_proof(proof),
            42
        ));
        assert!(!verify_plonk(PairingCurve::Bn254, &proof, 42));
    }
}
