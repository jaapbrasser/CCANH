//! Minimal H.264 helpers: Annex-B parsing into access units, AVC (length-
//! prefixed) conversion, RFC 6381 codec strings and the MP4 `avc1`/`avcC`
//! sample entry.

use bytes::Bytes;

pub const NAL_IDR: u8 = 5;
pub const NAL_SPS: u8 = 7;
pub const NAL_PPS: u8 = 8;
pub const NAL_AUD: u8 = 9;

/// Split an Annex-B byte stream into NAL units (without start codes).
pub fn nal_units(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new(); // (start of start code, start of NAL)
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            let code_start = if i > 0 && data[i - 1] == 0 { i - 1 } else { i };
            starts.push((code_start, i + 3));
            i += 3;
        } else {
            i += 1;
        }
    }
    starts
        .iter()
        .enumerate()
        .map(|(k, &(_, nal_start))| {
            let end = starts.get(k + 1).map_or(data.len(), |&(next, _)| next);
            // Trailing zero bytes belong to the next start code / padding.
            let mut end = end;
            while end > nal_start && data[end - 1] == 0 {
                end -= 1;
            }
            &data[nal_start..end]
        })
        .filter(|n| !n.is_empty())
        .collect()
}

pub fn nal_type(nal: &[u8]) -> u8 {
    nal.first().map_or(0, |b| b & 0x1f)
}

/// One decoded-order access unit from an Annex-B stream.
#[derive(Debug, Clone)]
pub struct AccessUnit {
    pub keyframe: bool,
    /// AVC format, without SPS/PPS/AUD (those live in the sample entry).
    pub avc: Bytes,
}

/// Parsed Annex-B elementary stream: parameter sets plus access units.
#[derive(Debug, Clone)]
pub struct ElementaryStream {
    pub sps: Vec<u8>,
    pub pps: Vec<u8>,
    pub units: Vec<AccessUnit>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum H264Error {
    #[error("no SPS/PPS in stream")]
    MissingParameterSets,
    #[error("no access units in stream")]
    Empty,
    #[error("stream does not start with a keyframe")]
    NoLeadingKeyframe,
}

/// Parse an Annex-B stream that uses access unit delimiters (one AUD per
/// frame), as produced by `scripts/gen-demo-clips.ps1`.
pub fn parse_annexb(data: &[u8]) -> Result<ElementaryStream, H264Error> {
    let mut sps = None;
    let mut pps = None;
    let mut units = Vec::new();
    let mut current: Vec<&[u8]> = Vec::new();
    let flush = |current: &mut Vec<&[u8]>, units: &mut Vec<AccessUnit>| {
        if current.is_empty() {
            return;
        }
        let keyframe = current.iter().any(|n| nal_type(n) == NAL_IDR);
        units.push(AccessUnit {
            keyframe,
            avc: to_avc(current.iter().copied()).into(),
        });
        current.clear();
    };
    for nal in nal_units(data) {
        match nal_type(nal) {
            NAL_AUD => flush(&mut current, &mut units),
            NAL_SPS => {
                sps.get_or_insert_with(|| nal.to_vec());
            }
            NAL_PPS => {
                pps.get_or_insert_with(|| nal.to_vec());
            }
            _ => current.push(nal),
        }
    }
    flush(&mut current, &mut units);
    let (Some(sps), Some(pps)) = (sps, pps) else {
        return Err(H264Error::MissingParameterSets);
    };
    match units.first() {
        None => Err(H264Error::Empty),
        Some(u) if !u.keyframe => Err(H264Error::NoLeadingKeyframe),
        Some(_) => Ok(ElementaryStream { sps, pps, units }),
    }
}

/// Concatenate NAL units with 4-byte big-endian length prefixes.
pub fn to_avc<'a>(nals: impl IntoIterator<Item = &'a [u8]>) -> Vec<u8> {
    let mut out = Vec::new();
    for nal in nals {
        out.extend_from_slice(&u32::try_from(nal.len()).unwrap_or(u32::MAX).to_be_bytes());
        out.extend_from_slice(nal);
    }
    out
}

/// `avc1.PPCCLL` from the SPS profile, constraint flags and level bytes.
pub fn rfc6381(sps: &[u8]) -> String {
    match sps {
        [_, profile, constraints, level, ..] => {
            format!("avc1.{profile:02X}{constraints:02X}{level:02X}")
        }
        _ => "avc1.42E01E".to_owned(),
    }
}

fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(
        &u32::try_from(8 + body.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

/// `AVCDecoderConfigurationRecord` (ISO/IEC 14496-15) with one SPS and PPS.
pub fn avcc(sps: &[u8], pps: &[u8]) -> Vec<u8> {
    let mut r = vec![
        1,
        sps.get(1).copied().unwrap_or(0x4d),
        sps.get(2).copied().unwrap_or(0),
        sps.get(3).copied().unwrap_or(0x1f),
        0xff, // 4-byte NAL lengths
        0xe1, // one SPS
    ];
    r.extend_from_slice(&u16::try_from(sps.len()).unwrap_or(0).to_be_bytes());
    r.extend_from_slice(sps);
    r.push(1); // one PPS
    r.extend_from_slice(&u16::try_from(pps.len()).unwrap_or(0).to_be_bytes());
    r.extend_from_slice(pps);
    r
}

/// MP4 `avc1` visual sample entry wrapping `avcC`.
pub fn avc1_sample_entry(width: u32, height: u32, sps: &[u8], pps: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&[0; 6]); // reserved
    b.extend_from_slice(&1u16.to_be_bytes()); // data_reference_index
    b.extend_from_slice(&[0; 16]); // pre_defined + reserved
    b.extend_from_slice(&u16::try_from(width).unwrap_or(0).to_be_bytes());
    b.extend_from_slice(&u16::try_from(height).unwrap_or(0).to_be_bytes());
    b.extend_from_slice(&0x0048_0000u32.to_be_bytes()); // 72 dpi
    b.extend_from_slice(&0x0048_0000u32.to_be_bytes());
    b.extend_from_slice(&[0; 4]); // reserved
    b.extend_from_slice(&1u16.to_be_bytes()); // frame_count
    b.extend_from_slice(&[0; 32]); // compressorname
    b.extend_from_slice(&0x0018u16.to_be_bytes()); // depth
    b.extend_from_slice(&(-1i16).to_be_bytes()); // pre_defined
    b.extend_from_slice(&mp4_box(b"avcC", &avcc(sps, pps)));
    mp4_box(b"avc1", &b)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIP: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../assets/demo/testsrc2-main.h264"
    ));

    #[test]
    fn splits_nal_units_with_3_and_4_byte_start_codes() {
        let data = [
            0, 0, 0, 1, 0x09, 0xf0, 0, 0, 1, 0x67, 1, 2, 0, 0, 0, 1, 0x68, 3,
        ];
        let nals = nal_units(&data);
        assert_eq!(
            nals,
            vec![&[0x09, 0xf0][..], &[0x67, 1, 2][..], &[0x68, 3][..]]
        );
    }

    #[test]
    fn parses_bundled_clip() {
        let es = parse_annexb(CLIP).unwrap();
        assert_eq!(es.units.len(), 150, "6 s at 25 fps");
        assert_eq!(es.units.iter().filter(|u| u.keyframe).count(), 3, "2 s GOP");
        assert!(es.units[0].keyframe);
        assert_eq!(rfc6381(&es.sps), "avc1.4D401F", "Main profile, level 3.1");
        // AVC units have no start codes.
        assert_ne!(&es.units[1].avc[..4], &[0, 0, 0, 1]);
    }

    #[test]
    fn sample_entry_layout() {
        let es = parse_annexb(CLIP).unwrap();
        let entry = avc1_sample_entry(1280, 720, &es.sps, &es.pps);
        assert_eq!(&entry[4..8], b"avc1");
        assert_eq!(
            u32::from_be_bytes(entry[..4].try_into().unwrap()) as usize,
            entry.len()
        );
        let pos = entry.windows(4).position(|w| w == b"avcC").unwrap();
        assert_eq!(entry[pos + 4], 1, "configurationVersion");
    }

    #[test]
    fn rejects_streams_without_parameter_sets() {
        assert_eq!(
            parse_annexb(&[0, 0, 1, 0x09, 0, 0, 1, 0x65, 1]).unwrap_err(),
            H264Error::MissingParameterSets
        );
    }
}
