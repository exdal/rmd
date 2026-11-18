pub fn f32(result: f32, operands: &[f32]) -> f32 {
    let is_fresh_nan = result.is_nan() && !operands.iter().any(|operand| operand.is_nan());
    if is_fresh_nan {
        return -f32::NAN;
    }

    result
}

#[cfg(test)]
mod tests {
    use super as number;

    #[test]
    fn fresh_nan_has_the_sign_bit_set() {
        let result = number::f32(f32::from_bits(0x7FC0_0000), &[f32::INFINITY, f32::INFINITY]);

        assert!(result.is_nan());
        assert!(result.is_sign_negative());
    }

    #[test]
    fn nan_operands_keep_their_bits() {
        let operand = f32::from_bits(0x7FC0_1234);
        let result = number::f32(f32::from_bits(0x7FC0_1234), &[operand, 1.0]);

        assert_eq!(result.to_bits(), 0x7FC0_1234);
    }

    #[test]
    fn finite_results_are_untouched() {
        assert_eq!(number::f32(3.0, &[1.0, 2.0]), 3.0);
        assert_eq!(number::f32(f32::INFINITY, &[f32::INFINITY, 1.0]), f32::INFINITY);
    }
}
