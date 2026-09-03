struct Sprite {
  float4 position_size;
  float4 color;
  int texture;
};

[[vk::binding(0, 1)]]
Sampler2D textures[];

[[vk::binding(1, 0)]]
StructuredBuffer<Sprite> sprites;

struct VSOutput {
  float4 position : SV_Position;
  float2 uv;
  nointerpolation uint texture_index;
  float4 color;
};

static const float2 CORNERS[4] = {
  float2(0.0, 0.0),
  float2(1.0, 0.0),
  float2(0.0, 1.0),
  float2(1.0, 1.0),
};

[shader("vertex")]
func vs_main(uint vertex_id: SV_VertexID, uint instance_id: SV_InstanceID,
             uniform float2 center, uniform float2 viewport, uniform float zoom)
    ->VSOutput {
  let sprite = sprites[instance_id];
  let corner = CORNERS[vertex_id];

  let map = sprite.position_size.xy + corner * sprite.position_size.zw;
  let view = (map - center) * zoom;
  let clip = view / (viewport * 0.5);

  VSOutput output;
  output.position = float4(clip.x, -clip.y, 0.0, 1.0);
  output.uv = float2(corner.x, 1.0 - corner.y);
  output.texture_index = sprite.texture;
  output.color = sprite.color;

  return output;
}

[shader("fragment")]
func fs_main(VSOutput input)->float4 {
  let texture_index = NonUniformResourceIndex(input.texture_index);
  let texel = textures[texture_index].Sample(input.uv) * input.color;

  // BYOND:tm: special
  return float4(texel.rgb * texel.a, texel.a);
}
