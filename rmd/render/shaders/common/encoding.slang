implementing common;

public func meshopt_dequantize_half(u32 half) -> f32 {
    const let sign = (half & 0x8000) << 16;
    const let exponent_mantissa = half & 0x7fff;

    var result = (exponent_mantissa + (112 << 10)) << 13;
    result = (exponent_mantissa < (1 << 10)) ? 0 : result;
    result += (exponent_mantissa >= (31 << 10)) ? (112 << 23) : 0;

    return asfloat(sign | result);
}

public func unpack_half2(u32 packed) -> f32x2 {
    return f32x2(meshopt_dequantize_half(packed & 0xffff), meshopt_dequantize_half(packed >> 16));
}

public func meshopt_quantize_unorm(f32 value, u32 bits) -> u32 {
    const let scale = f32((1 << bits) - 1);
    return u32(value * scale + 0.5);
}

public func meshopt_dequantize_unorm(u32 value, u32 bits) -> f32 {
    return f32(value) / f32((1 << bits) - 1);
}

public func pack_unorm4x8(f32x4 values) -> u32 {
    const let saturated = saturate(values);
    return meshopt_quantize_unorm(saturated.x, 8)
        | (meshopt_quantize_unorm(saturated.y, 8) << 8)
        | (meshopt_quantize_unorm(saturated.z, 8) << 16)
        | (meshopt_quantize_unorm(saturated.w, 8) << 24);
}

public func unpack_unorm4x8(u32 packed) -> f32x4 {
    return f32x4(
        meshopt_dequantize_unorm(packed & 0xff, 8),
        meshopt_dequantize_unorm((packed >> 8) & 0xff, 8),
        meshopt_dequantize_unorm((packed >> 16) & 0xff, 8),
        meshopt_dequantize_unorm(packed >> 24, 8)
    );
}

public func unpack_u16x2(u32 packed) -> u32x2 {
    return u32x2(packed & 0xffff, packed >> 16);
}
