# niri · KalangoOS

Este é o [niri](https://github.com/YaLTeR/niri) (compositor Wayland em Rust, de Ivan Molodetskikh e
colaboradores) com os patches do [KalangoOS](https://github.com/kalangoos/kalangoos). O ramo `main` acompanha o
upstream; o ramo **`kalangoos`** é o niri v26.04 (`8ed0da4`) com um commit por patch:

| Commit | O quê |
|---|---|
| encaixe de janelas e teclas presas | Arrastar até a borda ou o canto encaixa em metades, quartos ou maximizado; ação `snap-window`; teclas soltas quando a janela aninhada perde o foco |
| miniaturas de janelas | Ação `ThumbnailWindow` (PNG reduzido da janela, para o Alt+Tab) |
| Super sozinho | Toque e segurar a tecla Super rodam comandos da sessão (busca e modo voz) |
| posição das janelas lado a lado na IPC | `tile_pos_in_workspace_view` também para janelas em coluna |
| encaixe nas bordas opcional | Ação `SetEdgeSnap { on }` |
| minimizar | `xdg set_minimized`, ação `minimize-window`, IPC `is_minimized` |

Os mesmos patches estão em `sessao/niri/` no repositório do KalangoOS, aplicados por `sessao/niri/preparar.sh`.
Ideias que servem a todos podem ser propostas ao upstream; o resto fica aqui.

Licença: **GPL-3.0-or-later**, como o niri ([LICENSE](LICENSE)). Defeitos destes patches: abra a issue em
[kalangoos/kalangoos](https://github.com/kalangoos/kalangoos/issues), não no niri.
