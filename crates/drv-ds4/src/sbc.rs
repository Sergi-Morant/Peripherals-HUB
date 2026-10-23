//! Minimal SBC encoder for the DualShock 4 speaker: 32 kHz, dual channel, 8 subbands, 16 blocks,
//! loudness allocation, bitpool 25. That fixed layout makes every frame 112 bytes, and the pad
//! takes two of them in each Bluetooth audio report. Follows the A2DP specification (appendix B).

#![allow(clippy::needless_range_loop)] // DSP code reads clearer with explicit indices

pub const BLOCKS: usize = 16;
pub const SUBBANDS: usize = 8;
pub const BITPOOL: usize = 25;
/// Samples per channel in one frame.
pub const FRAME_SAMPLES: usize = BLOCKS * SUBBANDS;
pub const FRAME_BYTES: usize = 4 + SUBBANDS + BLOCKS * BITPOOL * 2 / 8;

/// Offsets of the loudness allocation for 32 kHz and 8 subbands.
const OFFSET: [i32; 8] = [-3, 0, 0, 0, 0, 0, 1, 2];

/// The 80-tap prototype filter of the 8-subband analysis, with the sign changes that go with the
/// cosine matrix below (taps 4, 13, 14 and 15 of every 16 are negated in both, which cancels out
/// but lets the table stay smooth). Same values as libsbc.
const PROTO: [f64; 80] = [
    0.00000000e0, 1.56575398e-04, 3.43256425e-04, 5.54620202e-04,
    -8.23919506e-04, 1.13992507e-03, 1.47640169e-03, 1.78371725e-03,
    2.01182542e-03, 2.10371989e-03, 1.99454554e-03, 1.61656283e-03,
    9.02154502e-04, 1.78805361e-04, 1.64973098e-03, 3.49717454e-03,
    5.65949473e-03, 8.02941163e-03, 1.04584443e-02, 1.27472335e-02,
    -1.46525263e-02, 1.59045603e-02, 1.62208471e-02, 1.53184106e-02,
    1.29371806e-02, 8.85757540e-03, 2.92408442e-03, -4.91578024e-03,
    -1.46404076e-02, 2.61098752e-02, 3.90751381e-02, 5.31873032e-02,
    6.79989431e-02, 8.29847578e-02, 9.75753918e-02, 1.11196689e-01,
    -1.23264548e-01, 1.33264415e-01, 1.40753505e-01, 1.45389847e-01,
    1.46955068e-01, 1.45389847e-01, 1.40753505e-01, 1.33264415e-01,
    1.23264548e-01, -1.11196689e-01, -9.75753918e-02, -8.29847578e-02,
    -6.79989431e-02, -5.31873032e-02, -3.90751381e-02, -2.61098752e-02,
    1.46404076e-02, -4.91578024e-03, 2.92408442e-03, 8.85757540e-03,
    1.29371806e-02, 1.53184106e-02, 1.62208471e-02, 1.59045603e-02,
    1.46525263e-02, -1.27472335e-02, -1.04584443e-02, -8.02941163e-03,
    -5.65949473e-03, -3.49717454e-03, -1.64973098e-03, -1.78805361e-04,
    -9.02154502e-04, 1.61656283e-03, 1.99454554e-03, 2.10371989e-03,
    2.01182542e-03, 1.78371725e-03, 1.47640169e-03, 1.13992507e-03,
    8.23919506e-04, -5.54620202e-04, -3.43256425e-04, -1.56575398e-04,
];

pub struct Encoder {
        cos: [[f64; 16]; 8],
    /// Analysis window per channel, newest sample first.
    x: [[f64; 80]; 2],
}

impl Default for Encoder {
    fn default() -> Self {
        let mut cos = [[0.0; 16]; 8];
        for (k, row) in cos.iter_mut().enumerate() {
            for (i, c) in row.iter_mut().enumerate() {
                let sign = if matches!(i, 4 | 13 | 14 | 15) { -1.0 } else { 1.0 };
                *c = sign * ((k as f64 + 0.5) * (i as f64 - 4.0) * std::f64::consts::PI / 8.0).cos();
            }
        }
        Encoder { cos, x: [[0.0; 80]; 2] }
    }
}

fn crc8(data: &[u8], bits: usize) -> u8 {
    let mut crc = 0x0Fu8;
    for i in 0..bits {
        let bit = (data[i / 8] >> (7 - i % 8)) & 1;
        crc = if (crc >> 7) ^ bit != 0 { (crc << 1) ^ 0x1D } else { crc << 1 };
    }
    crc
}

fn allocate(scale: &[u32; 8]) -> [usize; 8] {
    let mut need = [0i32; 8];
    let mut max = 0;
    for sb in 0..8 {
        need[sb] = if scale[sb] == 0 {
            -5
        } else {
            let l = scale[sb] as i32 - OFFSET[sb];
            if l > 0 { l / 2 } else { l }
        };
        max = max.max(need[sb]);
    }
    let pool = BITPOOL as i32;
    let (mut count, mut slice_count, mut slice) = (0, 0, max + 1);
    loop {
        slice -= 1;
        count += slice_count;
        slice_count = 0;
        for &n in &need {
            if n > slice + 1 && n < slice + 16 {
                slice_count += 1;
            } else if n == slice + 1 {
                slice_count += 2;
            }
        }
        if count + slice_count >= pool {
            break;
        }
    }
    if count + slice_count == pool {
        count += slice_count;
        slice -= 1;
    }
    let mut bits = [0i32; 8];
    for sb in 0..8 {
        bits[sb] = if need[sb] < slice + 2 { 0 } else { (need[sb] - slice).min(16) };
    }
    for sb in 0..8 {
        if count >= pool {
            break;
        }
        if bits[sb] >= 2 && bits[sb] < 16 {
            bits[sb] += 1;
            count += 1;
        } else if need[sb] == slice + 1 && pool > count + 1 {
            bits[sb] = 2;
            count += 2;
        }
    }
    for sb in 0..8 {
        if count >= pool {
            break;
        }
        if bits[sb] < 16 {
            bits[sb] += 1;
            count += 1;
        }
    }
    bits.map(|b| b as usize)
}

struct Writer<'a> {
    out: &'a mut [u8],
    pos: usize,
}

impl Writer<'_> {
    fn put(&mut self, value: u32, bits: usize) {
        for i in (0..bits).rev() {
            if (value >> i) & 1 != 0 {
                self.out[self.pos / 8] |= 0x80 >> (self.pos % 8);
            }
            self.pos += 1;
        }
    }
}

impl Encoder {
    /// Encode 128 stereo frames (`pcm` is interleaved L/R, 256 values) into one SBC frame.
    pub fn encode(&mut self, pcm: &[i16]) -> [u8; FRAME_BYTES] {
        assert_eq!(pcm.len(), FRAME_SAMPLES * 2);
        // Subband samples [block][channel][subband], in PCM units.
        let mut sb = [[[0.0f64; 8]; 2]; BLOCKS];
        for blk in 0..BLOCKS {
            for ch in 0..2 {
                let x = &mut self.x[ch];
                x.copy_within(0..72, 8);
                for i in 0..8 {
                    // sample 0 of the block is the oldest, so it ends at the highest slot
                    x[7 - i] = pcm[(blk * 8 + i) * 2 + ch] as f64;
                }
                let mut y = [0.0f64; 16];
                for (i, yi) in y.iter_mut().enumerate() {
                    *yi = (0..5).map(|j| PROTO[i + 16 * j] * x[i + 16 * j]).sum();
                }
                for k in 0..8 {
                    sb[blk][ch][k] = (0..16).map(|i| self.cos[k][i] * y[i]).sum();
                }
            }
        }
        let mut scale = [[0u32; 8]; 2];
        for ch in 0..2 {
            for k in 0..8 {
                let peak = (0..BLOCKS).map(|b| sb[b][ch][k].abs()).fold(0.0, f64::max);
                // smallest n with peak <= 2^(n+1)
                let mut sf = 0;
                while sf < 15 && peak > (2u32 << sf) as f64 {
                    sf += 1;
                }
                scale[ch][k] = sf;
            }
        }
        let mut frame = [0u8; FRAME_BYTES];
        frame[0] = 0x9C;
        frame[1] = 0x40 | 0x30 | 0x04 | 0x01; // 32 kHz, 16 blocks, dual channel, loudness, 8 subbands
        frame[2] = BITPOOL as u8;
        {
            let mut w = Writer { out: &mut frame, pos: 32 };
            for ch in 0..2 {
                for k in 0..8 {
                    w.put(scale[ch][k], 4);
                }
            }
        }
        let mut crc_in = [0u8; 10];
        crc_in[0] = frame[1];
        crc_in[1] = frame[2];
        crc_in[2..10].copy_from_slice(&frame[4..12]);
        frame[3] = crc8(&crc_in, 16 + 64);
        let bits = [allocate(&scale[0]), allocate(&scale[1])];
        let mut w = Writer { out: &mut frame, pos: 96 };
        for blk in 0..BLOCKS {
            for ch in 0..2 {
                for k in 0..8 {
                    let b = bits[ch][k];
                    if b == 0 {
                        continue;
                    }
                    let levels = ((1u32 << b) - 1) as f64;
                    let unit = (2u32 << scale[ch][k]) as f64; // 2^(sf+1)
                    let q = ((sb[blk][ch][k] / unit + 1.0) * levels / 2.0).floor();
                    w.put(q.clamp(0.0, levels) as u32, b);
                }
            }
        }
        frame
    }
}



#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f64, amp: f64) -> Vec<i16> {
        (0..FRAME_SAMPLES * 8)
            .flat_map(|i| {
                let v = (amp * (std::f64::consts::TAU * hz * i as f64 / 32_000.0).sin()) as i16;
                [v, v]
            })
            .collect()
    }

    #[test]
    fn frame_has_the_expected_header_and_size() {
        assert_eq!(FRAME_BYTES, 112);
        let mut enc = Encoder::default();
        let frames: Vec<_> = sine(1500.0, 2048.0).chunks(FRAME_SAMPLES * 2).map(|c| enc.encode(c)).collect();
        // sync, 32 kHz / 16 blocks / dual channel / loudness / 8 subbands, bitpool 25
        assert_eq!(&frames[5][..3], &[0x9C, 0x75, 25]);
        // The tone sits in the lowest subband (scale factor 10 for this amplitude, as libsbc's
        // `sbcenc` also finds); the rest is leakage, left unset above subband 1.
        assert_eq!(frames[5][4] >> 4, 10);
        assert_eq!(&frames[5][6..8], &[0, 0]);
    }

    #[test]
    fn allocation_uses_the_whole_bitpool() {
        for sf in [[10, 7, 0, 0, 0, 0, 0, 0], [11, 8, 3, 3, 2, 1, 0, 0], [5; 8]] {
            assert_eq!(allocate(&sf).iter().sum::<usize>(), BITPOOL);
        }
        // Checked against libsbc: the 32 kHz offset table makes this [14, 11].
        assert_eq!(&allocate(&[11, 8, 0, 0, 0, 0, 0, 0])[..2], &[14, 11]);
    }

    #[test]
    fn silence_encodes_without_panicking() {
        let mut enc = Encoder::default();
        let f = enc.encode(&vec![0i16; FRAME_SAMPLES * 2]);
        assert_eq!(f[4..12], [0; 8]);
    }
}
