// Scalar Shader Model 3 ports of FidelityFX FSR 1 EASU and RCAS.
// Algorithm reference: AMD FidelityFX FSR, MIT license.

pub const EASU: &str = r#"
sampler2D Source : register(s0);
float4 Params : register(c0); // input size xy, reciprocal input size zw

float3 Load(float2 pixel) {
    return tex2Dlod(Source, float4((pixel + 0.5) * Params.zw, 0.0, 0.0)).rgb;
}

float Luma(float3 color) {
    return color.g + 0.5 * (color.r + color.b);
}

void EasuSet(inout float2 dir, inout float len, float2 pp, float weight,
             float lA, float lB, float lC, float lD, float lE) {
    float dc = lD - lC;
    float cb = lC - lB;
    float lenX = rcp(max(max(abs(dc), abs(cb)), 1.0e-6));
    float dirX = lD - lB;
    dir.x += dirX * weight;
    lenX = saturate(abs(dirX) * lenX);
    len += lenX * lenX * weight;

    float ec = lE - lC;
    float ca = lC - lA;
    float lenY = rcp(max(max(abs(ec), abs(ca)), 1.0e-6));
    float dirY = lE - lA;
    dir.y += dirY * weight;
    lenY = saturate(abs(dirY) * lenY);
    len += lenY * lenY * weight;
}

void EasuTap(inout float3 color, inout float weight, float2 offset,
             float2 dir, float2 len, float lobe, float clipPoint, float3 tap) {
    float2 v;
    v.x = dot(offset, dir);
    v.y = dot(offset, float2(-dir.y, dir.x));
    v *= len;
    float distanceSquared = min(dot(v, v), clipPoint);
    float base = 0.4 * distanceSquared - 1.0;
    float window = lobe * distanceSquared - 1.0;
    base *= base;
    window *= window;
    base = 1.5625 * base - 0.5625;
    float tapWeight = base * window;
    color += tap * tapWeight;
    weight += tapWeight;
}

float4 main(float2 uv : TEXCOORD0) : COLOR0 {
    float2 position = uv * Params.xy - 0.5;
    float2 basePixel = floor(position);
    float2 pp = position - basePixel;

    float3 b = Load(basePixel + float2( 0.0, -1.0));
    float3 c = Load(basePixel + float2( 1.0, -1.0));
    float3 e = Load(basePixel + float2(-1.0,  0.0));
    float3 f = Load(basePixel + float2( 0.0,  0.0));
    float3 g = Load(basePixel + float2( 1.0,  0.0));
    float3 h = Load(basePixel + float2( 2.0,  0.0));
    float3 i = Load(basePixel + float2(-1.0,  1.0));
    float3 j = Load(basePixel + float2( 0.0,  1.0));
    float3 k = Load(basePixel + float2( 1.0,  1.0));
    float3 l = Load(basePixel + float2( 2.0,  1.0));
    float3 n = Load(basePixel + float2( 0.0,  2.0));
    float3 o = Load(basePixel + float2( 1.0,  2.0));

    float bL = Luma(b), cL = Luma(c), eL = Luma(e), fL = Luma(f);
    float gL = Luma(g), hL = Luma(h), iL = Luma(i), jL = Luma(j);
    float kL = Luma(k), lL = Luma(l), nL = Luma(n), oL = Luma(o);

    float2 dir = 0.0;
    float edgeLength = 0.0;
    EasuSet(dir, edgeLength, pp, (1.0 - pp.x) * (1.0 - pp.y), bL, eL, fL, gL, jL);
    EasuSet(dir, edgeLength, pp, pp.x * (1.0 - pp.y), cL, fL, gL, hL, kL);
    EasuSet(dir, edgeLength, pp, (1.0 - pp.x) * pp.y, fL, iL, jL, kL, nL);
    EasuSet(dir, edgeLength, pp, pp.x * pp.y, gL, jL, kL, lL, oL);

    float directionLengthSquared = dot(dir, dir);
    if (directionLengthSquared < (1.0 / 32768.0)) {
        dir = float2(1.0, 0.0);
    } else {
        dir *= rsqrt(directionLengthSquared);
    }

    edgeLength = edgeLength * 0.5;
    edgeLength *= edgeLength;
    float stretch = rcp(max(max(abs(dir.x), abs(dir.y)), 1.0e-6));
    float2 anisotropicLength = float2(
        1.0 + (stretch - 1.0) * edgeLength,
        1.0 - 0.5 * edgeLength
    );
    float lobe = 0.5 + (0.21 - 0.5) * edgeLength;
    float clipPoint = rcp(lobe);

    float3 minimum = min(min(f, g), min(j, k));
    float3 maximum = max(max(f, g), max(j, k));
    float3 accumulatedColor = 0.0;
    float accumulatedWeight = 0.0;
    EasuTap(accumulatedColor, accumulatedWeight, float2( 0.0, -1.0) - pp, dir, anisotropicLength, lobe, clipPoint, b);
    EasuTap(accumulatedColor, accumulatedWeight, float2( 1.0, -1.0) - pp, dir, anisotropicLength, lobe, clipPoint, c);
    EasuTap(accumulatedColor, accumulatedWeight, float2(-1.0,  1.0) - pp, dir, anisotropicLength, lobe, clipPoint, i);
    EasuTap(accumulatedColor, accumulatedWeight, float2( 0.0,  1.0) - pp, dir, anisotropicLength, lobe, clipPoint, j);
    EasuTap(accumulatedColor, accumulatedWeight, float2( 0.0,  0.0) - pp, dir, anisotropicLength, lobe, clipPoint, f);
    EasuTap(accumulatedColor, accumulatedWeight, float2(-1.0,  0.0) - pp, dir, anisotropicLength, lobe, clipPoint, e);
    EasuTap(accumulatedColor, accumulatedWeight, float2( 1.0,  1.0) - pp, dir, anisotropicLength, lobe, clipPoint, k);
    EasuTap(accumulatedColor, accumulatedWeight, float2( 2.0,  1.0) - pp, dir, anisotropicLength, lobe, clipPoint, l);
    EasuTap(accumulatedColor, accumulatedWeight, float2( 2.0,  0.0) - pp, dir, anisotropicLength, lobe, clipPoint, h);
    EasuTap(accumulatedColor, accumulatedWeight, float2( 1.0,  0.0) - pp, dir, anisotropicLength, lobe, clipPoint, g);
    EasuTap(accumulatedColor, accumulatedWeight, float2( 1.0,  2.0) - pp, dir, anisotropicLength, lobe, clipPoint, o);
    EasuTap(accumulatedColor, accumulatedWeight, float2( 0.0,  2.0) - pp, dir, anisotropicLength, lobe, clipPoint, n);

    return float4(clamp(accumulatedColor / accumulatedWeight, minimum, maximum), 1.0);
}
"#;

pub const RCAS: &str = r#"
sampler2D Source : register(s0);
float4 Params : register(c0); // reciprocal output size xy, sharpness coefficient z

float Luma(float3 color) {
    return color.g + 0.5 * (color.r + color.b);
}

float4 main(float2 uv : TEXCOORD0) : COLOR0 {
    float2 texel = Params.xy;
    float3 b = tex2D(Source, uv + float2(0.0, -texel.y)).rgb;
    float3 d = tex2D(Source, uv + float2(-texel.x, 0.0)).rgb;
    float3 e = tex2D(Source, uv).rgb;
    float3 f = tex2D(Source, uv + float2(texel.x, 0.0)).rgb;
    float3 h = tex2D(Source, uv + float2(0.0, texel.y)).rgb;

    float bL = Luma(b), dL = Luma(d), eL = Luma(e), fL = Luma(f), hL = Luma(h);
    float noise = abs(0.25 * (bL + dL + fL + hL) - eL);
    float maximumLuma = max(max(max(bL, dL), max(eL, fL)), hL);
    float minimumLuma = min(min(min(bL, dL), min(eL, fL)), hL);
    noise = saturate(noise / max(maximumLuma - minimumLuma, 1.0e-4));
    noise = 1.0 - 0.5 * noise;

    float3 minimumRing = min(min(b, d), min(f, h));
    float3 maximumRing = max(max(b, d), max(f, h));
    float3 hitMinimum = min(minimumRing, e) / max(4.0 * maximumRing, 1.0e-4);
    float3 hitMaximum = (1.0 - max(maximumRing, e)) / min(4.0 * minimumRing - 4.0, -1.0e-4);
    float3 lobes = max(-hitMinimum, hitMaximum);
    float lobe = max(-0.1875, min(max(lobes.r, max(lobes.g, lobes.b)), 0.0));
    lobe *= Params.z * noise;

    return float4((lobe * (b + d + f + h) + e) / (4.0 * lobe + 1.0), 1.0);
}
"#;

pub const FXAA: &str = r#"
sampler2D Source : register(s0);
float4 Params : register(c0); // reciprocal output size xy, subpixel blend z, minimum threshold w

float Luma(float3 color) {
    return color.g + 0.5 * (color.r + color.b);
}

float4 main(float2 uv : TEXCOORD0) : COLOR0 {
    float2 texel = Params.xy;
    float3 center = tex2D(Source, uv).rgb;
    float3 northWest = tex2D(Source, uv + texel * float2(-1.0, -1.0)).rgb;
    float3 northEast = tex2D(Source, uv + texel * float2( 1.0, -1.0)).rgb;
    float3 southWest = tex2D(Source, uv + texel * float2(-1.0,  1.0)).rgb;
    float3 southEast = tex2D(Source, uv + texel * float2( 1.0,  1.0)).rgb;

    float lumaCenter = Luma(center);
    float lumaNorthWest = Luma(northWest);
    float lumaNorthEast = Luma(northEast);
    float lumaSouthWest = Luma(southWest);
    float lumaSouthEast = Luma(southEast);
    float lumaMinimum = min(lumaCenter, min(min(lumaNorthWest, lumaNorthEast), min(lumaSouthWest, lumaSouthEast)));
    float lumaMaximum = max(lumaCenter, max(max(lumaNorthWest, lumaNorthEast), max(lumaSouthWest, lumaSouthEast)));
    float contrast = lumaMaximum - lumaMinimum;
    if (contrast < max(Params.w, lumaMaximum * 0.125)) {
        return float4(center, 1.0);
    }

    float2 direction;
    direction.x = -((lumaNorthWest + lumaNorthEast) - (lumaSouthWest + lumaSouthEast));
    direction.y =  ((lumaNorthWest + lumaSouthWest) - (lumaNorthEast + lumaSouthEast));
    float directionReduce = max(
        (lumaNorthWest + lumaNorthEast + lumaSouthWest + lumaSouthEast) * (0.25 / 8.0),
        1.0 / 128.0
    );
    float reciprocalMinimum = 1.0 / (min(abs(direction.x), abs(direction.y)) + directionReduce);
    direction = clamp(direction * reciprocalMinimum, -8.0, 8.0) * texel;

    float3 resultA = 0.5 * (
        tex2D(Source, uv + direction * (1.0 / 3.0 - 0.5)).rgb +
        tex2D(Source, uv + direction * (2.0 / 3.0 - 0.5)).rgb
    );
    float3 resultB = resultA * 0.5 + 0.25 * (
        tex2D(Source, uv + direction * -0.5).rgb +
        tex2D(Source, uv + direction *  0.5).rgb
    );
    float lumaResultB = Luma(resultB);
    float3 filtered = (lumaResultB < lumaMinimum || lumaResultB > lumaMaximum) ? resultA : resultB;
    return float4(lerp(center, filtered, Params.z), 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use std::ffi::c_void;

    use windows::Win32::Graphics::Direct3D::Fxc::{D3DCOMPILE_OPTIMIZATION_LEVEL3, D3DCompile};
    use windows::Win32::Graphics::Direct3D::{ID3DBlob, ID3DInclude};
    use windows::core::{PCSTR, s};

    use super::*;

    #[test]
    fn shaders_compile_for_shader_model_3() {
        compile(EASU).expect("EASU must compile for ps_3_0");
        compile(RCAS).expect("RCAS must compile for ps_3_0");
        compile(FXAA).expect("FXAA must compile for ps_3_0");
    }

    fn compile(source: &str) -> Result<(), String> {
        let mut code: Option<ID3DBlob> = None;
        let mut errors: Option<ID3DBlob> = None;
        let result = unsafe {
            D3DCompile(
                source.as_ptr().cast::<c_void>(),
                source.len(),
                PCSTR::null(),
                None,
                None::<&ID3DInclude>,
                s!("main"),
                s!("ps_3_0"),
                D3DCOMPILE_OPTIMIZATION_LEVEL3,
                0,
                &mut code,
                Some(&mut errors),
            )
        };

        result.map_err(|error| {
            errors
                .as_ref()
                .map(|blob| {
                    let bytes = unsafe {
                        std::slice::from_raw_parts(
                            blob.GetBufferPointer().cast::<u8>(),
                            blob.GetBufferSize(),
                        )
                    };
                    String::from_utf8_lossy(bytes)
                        .trim_end_matches('\0')
                        .to_owned()
                })
                .unwrap_or_else(|| error.to_string())
        })
    }
}
