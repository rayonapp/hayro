use crate::filter::MAX_DECODED_LEN;
use crate::reader::Reader;
use alloc::vec;
use alloc::vec::Vec;

pub(crate) fn decode(data: &[u8]) -> Option<Vec<u8>> {
    let mut reader = Reader::new(data);
    let mut decoded = vec![];

    loop {
        if decoded.len() > MAX_DECODED_LEN {
            return None;
        }

        let length = reader.read_byte()?;

        match length {
            128 => break,
            0..=127 => {
                // PDFBOX-3990, just abort early if stream is invalid.
                let Some(bytes) = reader.read_bytes(length as usize + 1) else {
                    break;
                };

                decoded.extend(bytes);
            }
            _ => {
                let length = 257 - length as usize;
                decoded.extend([reader.read_byte()?].repeat(length));
            }
        }
    }

    Some(decoded)
}

#[cfg(test)]
mod tests {
    use crate::filter::run_length::decode;

    #[test]
    fn run_length_output_past_the_cap() {
        let input = [129, 0].repeat(crate::filter::MAX_DECODED_LEN / 128 + 2);
        assert!(decode(&input).is_none());
    }

    #[test]
    fn run_length() {
        let input = vec![4, 10, 11, 12, 13, 14, 253, 3, 128];
        assert_eq!(
            decode(&input).unwrap(),
            vec![10, 11, 12, 13, 14, 3, 3, 3, 3]
        );
    }
}
