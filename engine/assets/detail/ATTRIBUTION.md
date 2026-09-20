# Ground detail photo textures

CC0 public domain sets from Poly Haven (https://polyhaven.com/textures),
downloaded 2026-09-19 as 2K JPEG diffuse plus OpenGL normal maps;
the meadow pair was replaced on 2026-09-20.
Used as tilable world-space detail for terrain materials; distribution
masks stay procedural/landform-driven.

| Material | Set | Authors | Source URL |
| --- | --- | --- | --- |
| meadow | Grass Ground | Charlotte Baglioni | https://polyhaven.com/a/grass_ground |
| rock | Rock Face | Greg Zaal, Dario Barresi | https://polyhaven.com/a/rock_face |
| snow | Snow 02 | Rob Tuytel | https://polyhaven.com/a/snow_02 |
| scree | Gravel Ground 01 | Rob Tuytel | https://polyhaven.com/a/gravel_ground_01 |

No other maps from these sets are shipped (roughness stays per-material).

The meadow scan supplies blade/thatch luminance and normals; the shader authors
the alpine green palette separately. It replaces the previous moss/gravel scan.
Grass Ground was published 2026-09-18 according to
[the asset API](https://api.polyhaven.com/info/grass_ground).
[CC0 terms](https://polyhaven.com/license) and
[download metadata](https://api.polyhaven.com/files/grass_ground) accessed
2026-09-20. Upstream JPEGs are shipped without image modification:

- [Diffuse, 2K](https://dl.polyhaven.org/file/ph-assets/Textures/jpg/2k/grass_ground/grass_ground_diff_2k.jpg), MD5 `fd97f2402677e6686b2d9159fc3a4256`.
- [OpenGL normal, 2K](https://dl.polyhaven.org/file/ph-assets/Textures/jpg/2k/grass_ground/grass_ground_nor_gl_2k.jpg), MD5 `c8b2dd95f09868c4aacb91f62c673344`.
