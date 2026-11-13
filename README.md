# wxgeo

微信内置浏览器及网页定位伪装工具：本机 MITM 代理，对目标站点 HTML 注入定位脚本，代理 `navigator.geolocation` 和 `wx.getLocation` 的返回坐标。支持 macOS、Linux、Windows。

仅供学习交流，不建议用于问卷星位置签到等非法用途。

## 安装

```
cargo install wxgeo          # 从 crates.io
cargo install --git https://github.com/Temsys-Shen/wxgeo   # 从源码
```

或从 [Releases](https://github.com/Temsys-Shen/wxgeo/releases) 下载预编译包。

## 构建

```
cargo build --release
```

产物：`target/release/wxgeo`。

## 使用

```
wxgeo                    # TUI
wxgeo --plain            # 文本界面
wxgeo --headless         # 前台运行并打印日志
wxgeo --selftest [URL]   # 自检
wxgeo --update-campus-db # 更新坐标数据库
```

Linux 安装 CA 需要 root：`sudo wxgeo`。

### 按键

| 键 | 作用 |
| --- | --- |
| `s` | 一键开启或关闭：代理、CA 信任、系统代理 |
| `c` | 坐标：自定义、高校、地图选点 |
| `d` | 目标域名 |
| `a` | 全站模式 |
| `j` | 自定义 JS |
| `r` | 重启代理 |
| `l` | 清屏 |
| `q` | 退出并清理 |

## 坐标来源

- 内置高校校区库，数据来自 Wikidata
- 地图选点：浏览器中点击或搜索后确认回填
- 手工输入经纬度

## 配置

| 路径 | 内容 |
| --- | --- |
| `~/.wxgeo/config.json` | 坐标、域名、端口等 |
| `~/.wxgeo/campuses.json` | 自定义或更新的校区数据 |
| `~/.wxgeo/ca/` | 本地 CA 与缓存 |


## License

MIT
