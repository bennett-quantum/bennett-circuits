//! Domain-separated, length-delimited SHA-256 chain over exact UTF-8 lines.
//! This is transport self-consistency, not an attestation of target equivalence.
use super::*;
pub(super) struct Chain {
    value: [u8; 32],
    buffer: Vec<u8>,
}
impl Chain {
    pub fn new(capacity: usize) -> Result<Self> {
        let mut buffer = vec![];
        buffer
            .try_reserve(
                capacity
                    .checked_add(40)
                    .ok_or("QASM: digest buffer overflow")?,
            )
            .map_err(|e| Error(format!("QASM: allocate digest buffer: {e}")))?;
        Ok(Self {
            value: crate::sha256::sha256(b"bennett-qasm-sha256-chain-v1"),
            buffer,
        })
    }
    pub fn feed(&mut self, line: &[u8]) -> Result<()> {
        self.buffer.clear();
        self.buffer
            .try_reserve(
                40usize
                    .checked_add(line.len())
                    .ok_or("QASM: digest line overflow")?,
            )
            .map_err(|e| Error(format!("QASM: allocate digest buffer: {e}")))?;
        self.buffer.extend_from_slice(&self.value);
        self.buffer
            .extend_from_slice(&(line.len() as u64).to_be_bytes());
        self.buffer.extend_from_slice(line);
        self.value = crate::sha256::sha256(&self.buffer);
        Ok(())
    }
    pub fn hex(&self) -> String {
        crate::sha256::hex(&self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allocation_overflow_is_an_error_and_line_boundaries_are_bound() {
        assert!(Chain::new(usize::MAX).is_err());
        let mut a = Chain::new(64).unwrap();
        a.feed(b"a\n").unwrap();
        a.feed(b"b\n").unwrap();
        let mut b = Chain::new(64).unwrap();
        b.feed(b"a\nb\n").unwrap();
        assert_ne!(a.hex(), b.hex());
    }
}
