//! Minimal fragmented-MP4 writer for one H.264 video track (ISO/IEC 14496-12).
//! Spike quality: no B-frames (no ctts / composition offsets), one sample per
//! fragment.

pub const TIMESCALE: u32 = 90_000;

fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&u32::try_from(8 + body.len()).unwrap().to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

fn full(kind: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(4 + body.len());
    b.push(version);
    b.extend_from_slice(&flags.to_be_bytes()[1..]);
    b.extend_from_slice(body);
    bx(kind, &b)
}

const MATRIX: [u32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];

fn matrix() -> Vec<u8> {
    MATRIX.iter().flat_map(|v| v.to_be_bytes()).collect()
}

/// `ftyp` + `moov` for one video track. `sample_entry` is a complete `avc1` box.
pub fn init_segment(width: u32, height: u32, sample_entry: &[u8]) -> Vec<u8> {
    let ftyp = bx(
        b"ftyp",
        &[b"isom".as_slice(), &0x200u32.to_be_bytes(), b"isom", b"iso6", b"avc1", b"mp41"].concat(),
    );

    let mut mvhd = Vec::new();
    mvhd.extend_from_slice(&[0; 8]); // creation, modification
    mvhd.extend_from_slice(&1000u32.to_be_bytes()); // timescale
    mvhd.extend_from_slice(&0u32.to_be_bytes()); // duration
    mvhd.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // rate
    mvhd.extend_from_slice(&0x0100u16.to_be_bytes()); // volume
    mvhd.extend_from_slice(&[0; 10]);
    mvhd.extend_from_slice(&matrix());
    mvhd.extend_from_slice(&[0; 24]); // pre_defined
    mvhd.extend_from_slice(&2u32.to_be_bytes()); // next_track_ID
    let mvhd = full(b"mvhd", 0, 0, &mvhd);

    let mut tkhd = Vec::new();
    tkhd.extend_from_slice(&[0; 8]);
    tkhd.extend_from_slice(&1u32.to_be_bytes()); // track_ID
    tkhd.extend_from_slice(&[0; 4]);
    tkhd.extend_from_slice(&0u32.to_be_bytes()); // duration
    tkhd.extend_from_slice(&[0; 8]);
    tkhd.extend_from_slice(&[0; 4]); // layer, alternate_group
    tkhd.extend_from_slice(&[0; 4]); // volume, reserved
    tkhd.extend_from_slice(&matrix());
    tkhd.extend_from_slice(&(width << 16).to_be_bytes());
    tkhd.extend_from_slice(&(height << 16).to_be_bytes());
    let tkhd = full(b"tkhd", 0, 0x7, &tkhd); // enabled | in_movie | in_preview

    let mut mdhd = Vec::new();
    mdhd.extend_from_slice(&[0; 8]);
    mdhd.extend_from_slice(&TIMESCALE.to_be_bytes());
    mdhd.extend_from_slice(&0u32.to_be_bytes());
    mdhd.extend_from_slice(&0x55c4u16.to_be_bytes()); // language "und"
    mdhd.extend_from_slice(&[0; 2]);
    let mdhd = full(b"mdhd", 0, 0, &mdhd);

    let hdlr = full(
        b"hdlr",
        0,
        0,
        &[&[0u8; 4][..], b"vide", &[0; 12], b"VideoHandler\0"].concat(),
    );
    let vmhd = full(b"vmhd", 0, 1, &[0; 8]);
    let dref = full(b"dref", 0, 0, &[&1u32.to_be_bytes()[..], &full(b"url ", 0, 1, &[])].concat());
    let dinf = bx(b"dinf", &dref);
    let stsd = full(b"stsd", 0, 0, &[&1u32.to_be_bytes()[..], sample_entry].concat());
    let empty = |k: &[u8; 4]| full(k, 0, 0, &0u32.to_be_bytes());
    let stsz = full(b"stsz", 0, 0, &[0u8; 8]);
    let stbl = bx(
        b"stbl",
        &[stsd, empty(b"stts"), empty(b"stsc"), stsz, empty(b"stco")].concat(),
    );
    let minf = bx(b"minf", &[vmhd, dinf, stbl].concat());
    let mdia = bx(b"mdia", &[mdhd, hdlr, minf].concat());
    let trak = bx(b"trak", &[tkhd, mdia].concat());

    let mut trex = Vec::new();
    trex.extend_from_slice(&1u32.to_be_bytes()); // track_ID
    trex.extend_from_slice(&1u32.to_be_bytes()); // default_sample_description_index
    trex.extend_from_slice(&[0; 12]); // default duration, size, flags
    let mvex = bx(b"mvex", &full(b"trex", 0, 0, &trex));

    let moov = bx(b"moov", &[mvhd, trak, mvex].concat());
    [ftyp, moov].concat()
}

/// `moof` + `mdat` for one sample. `data` is AVC (length-prefixed) access unit.
pub fn fragment(seq: u32, decode_time: u64, duration: u32, keyframe: bool, data: &[u8]) -> Vec<u8> {
    let mfhd = full(b"mfhd", 0, 0, &seq.to_be_bytes());
    // default-base-is-moof
    let tfhd = full(b"tfhd", 0, 0x02_0000, &1u32.to_be_bytes());
    let tfdt = full(b"tfdt", 1, 0, &decode_time.to_be_bytes());
    let sample_flags: u32 = if keyframe { 0x0200_0000 } else { 0x0101_0000 };

    // trun: data-offset | sample-duration | sample-size | sample-flags
    let trun_len = 12 + 4 + 4 + 12;
    let traf_len = 8 + tfhd.len() + tfdt.len() + trun_len;
    let moof_len = 8 + mfhd.len() + traf_len;
    let data_offset = u32::try_from(moof_len + 8).unwrap(); // past moof + mdat header
    let mut trun = Vec::new();
    trun.extend_from_slice(&1u32.to_be_bytes()); // sample_count
    trun.extend_from_slice(&data_offset.to_be_bytes());
    trun.extend_from_slice(&duration.to_be_bytes());
    trun.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
    trun.extend_from_slice(&sample_flags.to_be_bytes());
    let trun = full(b"trun", 0, 0x0701, &trun);
    debug_assert_eq!(trun.len(), trun_len);

    let traf = bx(b"traf", &[tfhd, tfdt, trun].concat());
    let moof = bx(b"moof", &[mfhd, traf].concat());
    debug_assert_eq!(moof.len(), moof_len);
    [moof, bx(b"mdat", data)].concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxes(mut b: &[u8]) -> Vec<(String, usize)> {
        let mut out = Vec::new();
        while b.len() >= 8 {
            let len = u32::from_be_bytes(b[..4].try_into().unwrap()) as usize;
            out.push((String::from_utf8_lossy(&b[4..8]).into_owned(), len));
            b = &b[len..];
        }
        out
    }

    #[test]
    fn fragment_layout_and_data_offset() {
        let data = [0, 0, 0, 2, 0x65, 0x88];
        let f = fragment(1, 0, 3600, true, &data);
        let top = boxes(&f);
        assert_eq!(top[0].0, "moof");
        assert_eq!(top[1].0, "mdat");
        assert_eq!(top[0].1 + top[1].1, f.len());
        // data_offset must point at the first sample byte.
        let off = f.windows(4).position(|w| w == b"trun").unwrap() + 4 + 4 + 4;
        let data_offset = u32::from_be_bytes(f[off..off + 4].try_into().unwrap()) as usize;
        assert_eq!(&f[data_offset..], &data);
    }

    #[test]
    fn init_has_ftyp_and_moov() {
        let top = boxes(&init_segment(1280, 720, &bx(b"avc1", &[0; 78])));
        assert_eq!(top.iter().map(|b| b.0.as_str()).collect::<Vec<_>>(), ["ftyp", "moov"]);
    }
}
