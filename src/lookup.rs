use crate::kzg10;
use crate::lookup_table::{LookUpTable, PreProcessedTable};
use crate::multiset::MultiSet;
use crate::multiset_equality;
use crate::proof::{Commitments, Evaluations, MultiSetEqualityProof};
use crate::quotient_poly;
use crate::transcript::TranscriptProtocol;
use algebra::bls12_381::Fr;
use algebra::Bls12_381;
use ff_fft::{DensePolynomial as Polynomial, EvaluationDomain};
use poly_commit::kzg10::Powers;
pub struct LookUp<T: LookUpTable> {
    table: T,
    // This is the set of values which we want to prove is a subset of the
    // table values. This may or may not be equal to the whole witness.
    left_wires: MultiSet,
    right_wires: MultiSet,
    output_wires: MultiSet,
}

impl<T: LookUpTable> LookUp<T> {
    pub fn new(table: T) -> LookUp<T> {
        LookUp {
            table: table,
            left_wires: MultiSet::new(),
            right_wires: MultiSet::new(),
            output_wires: MultiSet::new(),
        }
    }
    // First reads a value from the underlying table
    // Then we add the key and value to their respective multisets
    // Returns true if the value existed in the table
    pub fn read(&mut self, key: &(Fr, Fr)) -> bool {
        let option_output = self.table.read(key);
        if option_output.is_none() {
            return false;
        }
        let output = *option_output.unwrap();

        // Add (input, output) combination into the corresponding multisets
        self.left_wires.push(key.0);
        self.right_wires.push(key.1);
        self.output_wires.push(output);

        return true;
    }

    /// Aggregates the table and witness values into one multiset
    /// sorts, and pads the witness and or table to be the correct size
    pub fn to_multiset(
        &mut self,
        preprocessed_table: &PreProcessedTable,
        alpha: Fr,
    ) -> (MultiSet, MultiSet) {
        // Now we need to aggregate our table values into one multiset
        let mut merged_table = MultiSet::aggregate(
            vec![
                &preprocessed_table.t_1.0,
                &preprocessed_table.t_2.0,
                &preprocessed_table.t_3.0,
            ],
            alpha,
        );
        // Sort merged table values
        merged_table = merged_table.sort();

        // Pad left, right and output wires to be one less than the table multiset
        let pad_by = preprocessed_table.n - 1 - self.left_wires.len();
        self.left_wires.extend(pad_by, self.left_wires.last());

        self.right_wires.extend(pad_by, self.right_wires.last());

        self.output_wires.extend(pad_by, self.output_wires.last());

        // Now we need to aggregate our witness values into one multiset
        let merged_witness = MultiSet::aggregate(
            vec![&self.left_wires, &self.right_wires, &self.output_wires],
            alpha,
        );

        assert!(merged_witness.len() < merged_table.len()); // XXX: We could incorporate this in the API by counting the number of reads

        (merged_witness, merged_table)
    }

    /// Creates a proof that the multiset is within the table
    pub fn prove(
        &mut self,
        proving_key: &Powers<Bls12_381>,
        preprocessed_table: &PreProcessedTable,
        transcript: &mut dyn TranscriptProtocol,
    ) -> MultiSetEqualityProof {
        // Generate alpha challenge
        let alpha = transcript.challenge_scalar(b"alpha");
        transcript.append_scalar(b"alpha", &alpha);

        // Aggregate witness and table values using a random challenge
        let (f, t) = self.to_multiset(preprocessed_table, alpha);
        assert_eq!(f.len() + 1, t.len());

        // Create a Multi-set equality proof
        multiset_equality::prove(f, t, proving_key, transcript)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::lookup_table::XOR4BitTable;
    use merlin::Transcript;

    #[test]
    fn test_pad_correct() {
        // Setup SRS
        let (proving_key, _) = kzg10::trusted_setup(2usize.pow(12), b"insecure_seed");

        let table = XOR4BitTable::new();
        let preprocessed_table = table.preprocess(&proving_key, 2usize.pow(8));

        // Setup lookup and add 3 XOR reads into it
        let mut lookup = LookUp::new(table);

        // Add 1 XOR 2
        lookup.read(&(Fr::from(2u8), Fr::from(2u8)));
        // Add 2 XOR 4
        lookup.read(&(Fr::from(3u8), Fr::from(2u8)));
        // Add 3 XOR 5
        lookup.read(&(Fr::from(1u8), Fr::from(2u8)));

        let (f, t) = lookup.to_multiset(&preprocessed_table, Fr::from(5u8));
        assert_eq!(f.len() + 1, t.len());

        assert!(t.len().is_power_of_two());
    }

    #[test]
    fn test_inclusion() {
        // Setup SRS
        let (proving_key, _) = kzg10::trusted_setup(2usize.pow(12), b"insecure_seed");

        let table = XOR4BitTable::new();
        let preprocessed_table = table.preprocess(&proving_key, 2usize.pow(8));

        let mut lookup = LookUp::new(table);

        // Add 2 XOR 2
        lookup.read(&(Fr::from(2u8), Fr::from(2u8)));
        // Add 1 XOR 2
        lookup.read(&(Fr::from(1u8), Fr::from(2u8)));
        // Add 3 XOR 5
        lookup.read(&(Fr::from(1u8), Fr::from(2u8)));

        let (f, t) = lookup.to_multiset(&preprocessed_table, Fr::from(5u8));
        assert!(f.is_subset_of(&t));
    }
    #[test]
    fn test_len() {
        // Check that the correct values are being added to the witness
        // If the value is not in the XOR4BitTable, it is not added to the witness
        // For a 4-bit XOR table the range is [0,15]

        // Setup SRS
        let (proving_key, _) = kzg10::trusted_setup(2usize.pow(12), b"insecure_seed");

        let table = XOR4BitTable::new();
        let preprocessed_table = table.preprocess(&proving_key, 2usize.pow(8));

        let mut lookup = LookUp::new(table);

        let added = lookup.read(&(Fr::from(16u8), Fr::from(6u8)));
        assert!(!added);

        let added = lookup.read(&(Fr::from(8u8), Fr::from(17u8)));
        assert!(!added);
        let added = lookup.read(&(Fr::from(15u8), Fr::from(13u8)));
        assert!(added);

        assert_eq!(lookup.left_wires.len(), 1);
        assert_eq!(lookup.right_wires.len(), 1);
        assert_eq!(lookup.output_wires.len(), 1);

        let (f, t) = lookup.to_multiset(&preprocessed_table, Fr::from(5u8));
        assert!(f.is_subset_of(&t));
    }
    #[test]
    fn test_proof() {
        // Setup SRS
        let (proving_key, verifier_key) = kzg10::trusted_setup(2usize.pow(12), b"insecure_seed");

        // Setup Lookup with a 4 bit table
        let table = XOR4BitTable::new();
        let preprocessed_table = table.preprocess(&proving_key, 2usize.pow(8));

        let mut lookup = LookUp::new(table);

        // Adds 1 XOR 2
        lookup.read(&(Fr::from(1u8), Fr::from(2u8)));
        // Adds 2 XOR 4
        lookup.read(&(Fr::from(2u8), Fr::from(4u8)));
        // Adds 3 XOR 5
        lookup.read(&(Fr::from(3u8), Fr::from(5u8)));

        let mut prover_transcript = Transcript::new(b"lookup");
        let proof = lookup.prove(&proving_key, &preprocessed_table, &mut prover_transcript);

        let mut verifier_transcript = Transcript::new(b"lookup");
        let ok = proof.verify(&verifier_key, &preprocessed_table, &mut verifier_transcript);
        assert!(ok);
    }
}
