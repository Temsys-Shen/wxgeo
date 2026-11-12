# wxgeo

wxgeo 是面向微信内置浏览器（Chromium / flue 内核）的定位伪装工具。实现方式为本机 MITM 代理：仅对目标站点的 HTML 注入定位伪造脚本，其余流量原样透传。微信本体不做改动，内置浏览器保持可用。支持 macOS、Linux、Windows。

## 功能

- 目标站点可配置：域名列表或全站模式，按域名动态签发证书
- 定位可配置：纬度、经度、精度；内置全国高校校区库，可在界面检索选择，也可手工输入
- 注入覆盖 `navigator.geolocation` 与 `wx.getLocation`
- 站点特有校验可补充：单行自定义 JS 注入到 `<head>`
- 交互界面（crossterm TUI）：启停代理、改坐标、改域名、信任 CA、系统代理一键切换
- 无界面模式：`--plain` 文本界面、`--headless` 前台日志、`--selftest` 自检

## 构建

需要 Rust 稳定版工具链。

```
cargo build --release
./target/release/wxgeo
```

跨平台交叉构建由 CI 完成，产物见 Releases：

| 平台 | 产物 |
| --- | --- |
| macOS arm64 | wxgeo-macos-arm64.tar.gz |
| macOS x86_64 | wxgeo-macos-x86_64.tar.gz |
| Linux x86_64 | wxgeo-linux-x86_64.tar.gz |
| Windows x86_64 | wxgeo-windows-x86_64.zip |

## 使用

1. 运行 `wxgeo` 进入界面
2. 按 `t` 安装并信任本地 CA
3. 按 `c` 设定纬度、经度、精度
4. 按 `d` 填入目标域名，逗号分隔，例如 `wjx.top,wjx.cn`
5. 按 `p` 开启系统代理
6. 按 `s` 启动代理
7. 在微信内置浏览器打开目标页面
8. 结束后按 `s` 停止代理、按 `p` 关闭系统代理、按 `t` 移除 CA 信任

## 按键

| 键 | 作用 |
| --- | --- |
| `s` | 启动或停止代理 |
| `c` | 修改坐标与精度 |
| `d` | 修改目标域名列表 |
| `a` | 切换全站模式 |
| `j` | 设置自定义 JS |
| `u` | 检索并选择高校校区 |
| `t` | 安装或移除本地 CA 信任 |
| `p` | 开启或关闭系统代理 |
| `r` | 重启代理 |
| `l` | 清空日志 |
| `q` | 退出 |

## 高校校区库

内置全国高校与校区坐标，来源 Wikidata，覆盖院校与主要校区。界面按 `u` 输入关键词（校名、城市、校区）检索，选择序号即写入坐标；也可按 `c` 手工输入经纬度。

扩充数据：

- `wxgeo --update-campus-db`：从 Wikidata 拉取院校与校区坐标，合并后写入 `~/.wxgeo/campuses.json`
- 手工维护 `~/.wxgeo/campuses.json`，格式为数组，元素含 name、city、lat、lon

内置数据为离线快照，覆盖范围以 Wikidata 收录为准。需要更完整的校区数据时，在网络可用环境执行更新，或导入自有数据文件。

## 平台差异

| 事项 | macOS | Linux | Windows |
| --- | --- | --- | --- |
| CA 信任 | 登录钥匙串 security | `/usr/local/share/ca-certificates` 加 update-ca-certificates，需要 sudo | certutil -user -store Root |
| 系统代理 | networksetup | 自动识别：GNOME 系用 gsettings、KDE 用 kioslaverc、其它生成环境变量文件 | 注册表 Internet Settings（WinINET） |
| 证书生成 | rcgen（纯 Rust） | rcgen | rcgen |

Linux 代理后端按顺序选择：

1. 可用环境变量 WXGEO_PROXY_BACKEND=gnome 或 kde 或 env 强制指定。
2. 检测到 KDE Plasma：写 `~/.config/kioslaverc` 的 Proxy Settings，并发送 KIO 重载信号。
3. 检测到 GNOME 系且存在 `org.gnome.system.proxy` schema：用 gsettings。
4. 其它情况：生成 `~/.wxgeo/proxy-env.sh`，启动目标程序前 source 该文件。

部分应用既不读系统代理也不读环境变量，此时可在受控环境用 nftables 把 443 重定向到本机端口做透明代理，配合本工具。

## 命令行

- `wxgeo`：进入 TUI
- `wxgeo --plain`：文本界面
- `wxgeo --selftest [URL]`：自检，生成 CA、启动代理、以 curl 验证注入
- `wxgeo --headless`：前台运行代理并打印日志
- `wxgeo --update-campus-db`：更新高校校区库到 `~/.wxgeo/campuses.json`

## CI 与发布

- `.github/workflows/ci.yml`：推送到 main 或提交 PR 时，在 macOS、Linux、Windows 三平台执行构建与测试
- `.github/workflows/release.yml`：推送 `v*` 标签时，三平台构建并打包，自动创建 GitHub Release 并上传产物
- 发布步骤：`git tag v0.1.0 && git push origin v0.1.0`

## 配置与数据

- 配置：`~/.wxgeo/config.json`
- CA 与站点证书：`~/.wxgeo/ca/`（站点证书按需生成并缓存于内存）

## 注意

- 系统代理开启期间，全机 HTTP/HTTPS 流量经本机代理转发
- 本地 CA 信任会降低本机 TLS 校验强度，使用结束应移除
- 默认坐标：电子科技大学沙河校区（30.678232, 104.098004）
- 仅在自有或获授权的测试环境使用
