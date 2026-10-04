# aegis

基于信息论和 ABAC 理念的高校课程考勤系统

## 项目背景

传统的高校课程考勤方法长期处于难以兼顾「效率」和「准确率」的困境。例如：

1. 线下点名：「准确率」较高，但存在「代签」现象；「效率」低，耗时较长。
2. 定位签到：「效率」较高，但是准确率一般。既存在「假阴性」的问题（学生手机的定位信号不好），也存在「假阳性」的问题（安卓系统可伪造定位）。
3. 二维码/手势/口令密码签到：「效率」高，但极易被传播，准确率极低。
4. 动态二维码签到：给二维码设置极短的TTL（如5秒）并高频刷新二维码。但无法防御流媒体实时传输二维码画面。

除此之外，传统方法也难以解决「辨别早退」、「迟到界限模糊」（难以辨别迟到15分钟以内或以上）、「多机代签」等问题。

## 项目理念

### 关于「效率」的解决方案

我们提议全面线上化考勤，所有人可以并行签到，只花半分钟到一分钟即可完成签到。

### 关于「准确率」的解决方案

基于「信息论」，我们提议发明一种二维码图像加解密算法，被加密过的图像经过一般的视频编解码算法（如H.264，HEVC、AV1等）后会出现不可逆的信息损失，使得二维码无法被流媒体传播。

基于「ABAC理念」，我们提议为考勤引入多种认证因素：

1. Subject（主体）：引入「人脸活体检测」与「人脸识别」
2. Object（客体）：引入「二维码签到」
3. Action（行为）：对「签到」、「续签」、「签退」进行建模
4. Environment（环境）：引入「卫星定位」

### 关于「早退」、「迟到」、「请假」的解决方案

对于「早退」，由于效率的极大提高，教师可以毫无顾虑地在课间或课后发起「续签」和「签退」请求，让学生自助续签/签退。

对于「迟到」，允许只收集部分认证因素，稍后在课间找教师审核。

对于「请假」，允许学生在系统上发起申请并提供请假条凭据，由教师审核。

## 项目概要

1. 教师手动发起签到（设定签到位置、持续时间等），在教室投屏加密后的二维码，并采用短TTL（5秒）+高频刷新的方案。
2. 学生需要通过扫码+定位+人脸操作。若三项均通过则视为签到成功；若只有两项成功可发起申请+教师手动通过；若少于两项成功视为签到失败。
3. 教师可自由选择是否发起并手动发起续签和签退。发起续签和签退时，学生只需要进行定位和人脸操作，无需扫码
4. 课程结束后，只有所有环节（包括签到、续签、签退）都成功才视为考勤成功。
5. 对于二维码已结束放映后且迟到15分钟以内，可以只记录人脸+定位，随后向教师发起迟到审核。
6. 对于请假，在系统上发起申请并提供请假条凭据，通过后视为考勤成功。

## 项目结构

- `src`：Web 前端（React 19.3、React Router 8、TanStack Query、Orval、Base UI）
- `crates/core`：Rust，无密钥色度二维码编解码与人脸算法（二维码兼容 /Users/mioyi/Documents/Code/Python/qr-code-lab）
- `crates/wasm`：Rust，向前端导出色度二维码 PNG 编码与 RGBA 解码
- `crates/backend`：Rust，后端（Axum + utoipa），Auth使用session+单opaque token+RBAC

## 本地后端运行

已实现 `crates/core` 和 `crates/backend`；正常认证和推理完全离线。人脸推理由 Rust `tract-onnx` CPU 后端执行，图像预处理和检测后处理同样使用 Rust，不需要安装 OpenCV、ONNX Runtime 或 libclang 绑定生成工具。tract 自带优化汇编内核，构建仍需平台编译/汇编工具链（macOS 为 Command Line Tools），无需额外安装系统推理库。`aegis-core` 的 `native-face` feature 启用本地人脸推理；默认构建仍只包含可供 WASM 复用的 QR 核心。

```sh
sh scripts/fetch-models.sh models
RUST_LOG=info cargo run -p aegis-backend
curl --fail http://localhost:3000/readyz
curl --fail http://localhost:3000/api/openapi.json
```

首次模型准备需要 `curl`、`shasum`、Python 3 和网络。下载脚本使用固定版本，先核验大小、SHA-256 或 Git blob SHA-1，再安装文件并生成 `checksums.sha256`。每次启动重新校验四个模型，并对两个独立推理实例实际执行 forward；缺模型、摘要不匹配或配置无效时退出，不提供降级认证。模型和校验清单都应由可信管理员维护；清单不能抵御攻击者同时替换模型及清单。

| 环境变量 | 默认值 |
| --- | --- |
| `AEGIS_BIND` | `127.0.0.1:3000` |
| `DATABASE_URL` | `sqlite://data/aegis.db` |
| `AEGIS_ORIGIN` | `http://localhost:3000` |
| `AEGIS_COOKIE_SECURE` | `false` |
| `AEGIS_MODEL_DIR` | `models` |

SQLite 自动建库、迁移并创建父目录，启用 WAL、外键和 5 秒锁等待，连接池上限 8；锁等待超时返回 `503 busy`，不自动重试。两个独立人脸 worker 的等待队列容量为 16，满队列拒绝提交。公网部署必须采用 HTTPS、同源反向代理及 `AEGIS_COOKIE_SECURE=true`；`AEGIS_ORIGIN` 不含尾随 `/`。

### API 契约与输入

完整接口定义由 `GET /api/openapi.json` 提供，真实 HTTP 已核验 OpenAPI 3.1.0 的 26 个路径、29 个唯一操作及 cookie、Origin、分页、文件编码和错误契约。业务覆盖会话、教师课程与名单、课次阶段及投屏、学生人脸登记与考勤提交、人工审核、附件和最终汇总；`GET /healthz` 是存活检查，`GET /readyz` 检查数据库及已加载模型。列表 `courses/lessons/students/reviews` 采用 `{items,total}`，`limit=1..100`（默认 50）、`offset>=0`（默认 0）；考勤汇总返回完整可见快照，不分页。

登记 multipart 字段为 `challenge_id`、`frame_0..2`；考勤字段为 JSON 字符串 `payload`、可选 `frame_0..2`、常规签到才允许的 `qr_image`。JPEG/PNG 单图最多 2 MiB，解码宽高分别最多 1920，按 EXIF 方向处理；multipart 最多 12 MiB，JSON/文本字段最多 16 KiB。请假使用 `reason` 和 JPEG/PNG/PDF `evidence`，附件最多 5 MiB，校验 MIME 与魔数，仅本人及所属教师可下载。未知/重复字段及非法协议返回 `422 invalid_input`；过大输入返回 `413 invalid_input`。考勤请求不能指定 actor、因素通过值或服务端时间。

用户、考勤和审核响应不含密码摘要、session 摘要、人脸模板、模型分数或上传的认证原图；附件只在授权下载接口返回。首次登记后没有自助替换人脸接口。定位通过条件为 `distance + accuracy_m <= radius_m` 且 `0 < accuracy_m <= radius_m`；单帧身份 cosine 至少 0.363，双活体模型平均的真人概率至少 0.90，三帧均须通过。

### 回归命令

准备模型后运行以下回归命令，无需 OpenCV 环境变量：

```sh
cargo test -p aegis-core --test qr_contract
cargo test -p aegis-backend --test attendance_policy
cargo test -p aegis-backend --test review_transactions
cargo test -p aegis-backend --test session_access
cargo test --workspace
```

固定模型的上游许可证和署名保存在 `licenses/`，来源、版本及修正的 OpenCV Zoo commit 记录于 `licenses/SOURCES.txt`。YuNet 为 MIT；SFace、MiniFASNet 原项目和社区 ONNX 转换项目为 Apache-2.0。社区转换不代表上游官方认证。

## Web 前端

前端使用 React 19、React Router 8、TanStack Query 和 shadcn/ui 的 Base UI 组件，默认中文、浅色主题。教师维护课程、名单、课次与阶段，查看投屏、审核和完整冻结快照；学生使用移动端单列完成首次人脸登记、考勤、显式审核申请、请假和本人汇总。学生不能自主加课，页面不提供后端尚未支持的课程修改、资料编辑或人脸替换操作。

### 开发与构建

需要 Node.js ≥22.22、npm、Rust 和 `wasm32-unknown-unknown` target。`wasm-pack` 随开发依赖安装，不需要全局安装。先按上文准备模型和后端环境，再运行：

```sh
rustup target add wasm32-unknown-unknown
npm ci

# 后端终端：Origin 必须与浏览器页面源完全一致。
AEGIS_ORIGIN=http://localhost:5173 AEGIS_COOKIE_SECURE=false cargo run -p aegis-backend

# 前端终端：自动构建 QR WASM 后启动 Vite。
npm run dev
```

浏览器访问 **http://localhost:5173**，不要替换为 `127.0.0.1:5173`。dev 和 preview 都固定使用 5173，端口被占用时直接失败；`/api`、`/healthz`、`/readyz` 代理到 `http://127.0.0.1:3000`，保留浏览器 Origin。

OpenAPI 快照及 Orval 生成的客户端、类型已纳入源码，普通 dev/build 不要求后端在线生成客户端。后端契约变化时先启动后端，再执行：

```sh
npm run api:sync
npm run api:generate
# api:sync 可通过 AEGIS_OPENAPI_URL 指定文档地址。

cargo check -p aegis-wasm --target wasm32-unknown-unknown
npx wasm-pack test --node crates/wasm
cargo test --workspace
npm run build

# 停止 dev 后启动构建预览。
npm run preview
```

`cargo test --workspace` 不依赖系统图像或推理库。`npm run build` 依次构建 WASM、检查 TypeScript、输出 `dist/`；`node_modules/`、`dist/` 和 `src/wasm/generated/` 不纳入源码。

### WASM、媒体与会话边界

`crates/wasm` 直接复用纯 Rust QR 核心，导出 `encode_chroma_png` 和 `decode_chroma_rgba`，不启用 `native-face`，也不引入人脸模型推理依赖。编码仅接受 1–42 字符 ASCII；解码要求宽高各为 1–1920、RGBA 长度精确匹配，不可读图像返回 `undefined`。扫码在按需创建的 module worker 中运行，同一时刻只解一帧；识别后上传同一帧的无损 PNG，不上传解出的 token，不重绘教师返回的载波。

相机、定位仅在用户开始采集后启用；人脸三帧、扫码图和挑战只用于当前操作，不写入浏览器存储。取消、离页、退出和受保护请求的 401 会停止媒体及 worker，并清理账号缓存。提交不自动重试；二维码、人脸挑战过期、窗口关闭或 `503` 都不能降级为通过，重新开始会获取新挑战。

未送审的可审核回执只在当前 tab 的 `sessionStorage` 中保存最少元数据（账号、课次、阶段、回执 ID、服务端时间、因素及原因），刷新后仍须用户显式申请；不保存照片、坐标或挑战。申请送达、阶段通过或退出账号后移除回执。已送达申请以课次服务端审核记录为准，不承诺跨设备或关闭 tab 后恢复未送审明细。请假附件通过授权接口下载，按实际 MIME 保留扩展名，不公开附件地址或内联执行文件。

### 生产部署

使用 HTTPS 同源反向代理提供 `dist/` 静态资源及后端接口；SPA 深层页面路径回退 `index.html`，`/api`、`/healthz`、`/readyz` 交给后端。正确提供 module worker、字体及 `.wasm` 资源，WASM 的 Content-Type 为 `application/wasm`。设置 `AEGIS_ORIGIN` 为真实页面源（无尾随 `/`）、`AEGIS_COOKIE_SECURE=true`。Vite preview 仅用于本机构建验收，不作为生产服务器；不需要扩展 Axum 静态托管或开放跨源 API。

### 前端软件验收

真实后端与隔离数据库已完成注册、名单重复冲突、三因素签到、无二维码续签/签退、显式部分审核、迟到截止及审批、请假与附件越权、冻结历史名单、服务端第二页分页、权限拒绝、二维码/挑战过期、SQLite `503 busy`、提交前关闭课次，以及会话撤销和账号缓存隔离。PNG 附件下载字节与上传一致；2.7 MiB PDF 可上传并按 PDF 类型下载，超过 5 MiB 的凭据被拒绝。

构建预览已验证真实 module worker 与 `.wasm` 资源加载、深层路由刷新和三因素提交。学生 320/390px、教师 1440/768px 页面及 128 字符连续标题无页面横向溢出；导航、dialog、select 和 tab 的键盘操作、焦点恢复与 44px 触控目标已检查。投屏响应延迟超过 5 秒时不展示已过期 PNG，正常响应仍可刷新并进入全屏；TTL 估计计入请求及响应体交付耗时。退出接口真实返回 `503 busy` 时保留登录状态并允许手动重试，不假报退出成功。

软件烟测使用公开 T1 回归图的模拟相机、模拟定位和教师真实 QR 接口，所有业务响应和人脸推理来自真实后端，正式应用没有测试媒体入口。**尚未验证真实投影屏摄、手机定位准确度或自然人实时活体**；这些软件结果不证明防代签或防直播效果。验收数据库与媒体均位于独立临时目录，不修改既有数据库或模型。

## 本地后端安全边界

- 认证采用 SQLite 持久化 session：每账号仅一个 opaque cookie，固定七天过期，新登录撤销旧会话。密码使用带随机盐的 Argon2id；数据库仅保存 token 的 SHA-256 摘要。
- 所有变更请求（包括注册、登录、CLI 调用）必须发送与 `AEGIS_ORIGIN` 完全一致的 `Origin`。浏览器使用同源反向代理，不开放任意 CORS。
- 学生与教师均可自主注册；系统不证明自报教师身份或初次登记者的学籍身份，课程教师负责名单核对。
- 色度二维码是无密钥的易损载波，不是密码学加密；定位结果不能证明卫星数据真实，人脸被动活体和一次性 nonce 不能证明媒体来自新鲜摄像头。

## 已验证行为与边界

### 二维码

纯 Rust QR 核心经固定 Python 参考双向编解码验证；20 个 32 字符 token 的原载波和 `camera_photo(seed=0..19)` 模拟图均精确恢复 20/20。FFmpeg 9.0.2 下采样 4 倍、yuv420p，H.264（libx264/medium/crf28）、HEVC（libx265/medium/crf30）、AV1（libsvtav1/preset8/crf35）各精确恢复 0/20。此结果仅覆盖这些固定模拟参数，**未验证真实屏摄与会议转播**，不保证所有视频链路均无法传播。

### 人脸

本地人脸核心已迁移至 `tract-onnx 0.23.8`，在 macOS arm64、Rust 1.99.0 上实际加载四个固定模型并执行 CPU forward。模型文件、`MODEL_ID`、128 维归一化模板以及 cosine 0.363 / 活体 0.90 阈值不变。YuNet 按原图尺寸补零到 32 的倍数，使用 BGR 原值输入并执行检测解码/NMS；SFace 使用五点相似变换对齐后的 RGB 原值；双 MiniFASNet 使用原有尺度及边界平移的 BGR 裁剪。模型不做重写或重新下载，导入时从算子和实际输入重新推导形状，避免 YuNet 的固定导出注释限制原图尺寸；每个网络复用推理状态，检测器只保留最近一个尺寸的执行计划。

迁移烟测中，公开 T1 样本重复三次登记、本人验证及 OpenCV 参考模板验证均通过；同一 T1 的 Rust / OpenCV 5.0.0 参考特征 cosine 为 0.99973894，非逐位相同。F1/F2 均返回 `Spoof`，白图、非 32 倍数尺寸的白图及双人拼图均返回 `NotSingleFace`。裁剪边界浮点残差在整数截断后校验，与原有裁剪规则一致；NMS、对齐和裁剪边界有独立回归测试。样本遵守 JPEG EXIF 朝向；这些检查证明本地推理与策略集成，不证明新鲜摄像头输入、所有历史模板的兼容性或两个不同自然人的误识别率。

真实 HTTP 集成的多个隔离测试账号使用同一公开 T1 回归输入，**不是不同自然人或实时活体验收**。尚无两名同意者各至少三张独立照片，未验证这项物理边界。合法极小图片（1×1、1×80、80×1、2×2、31×31）已确认是 `face=false / face_not_single`，不是 `503 factor_unavailable`；二维码及定位通过时仍可显式申请部分审核，登记此类图片返回 422。

### 业务状态

教师维护课程名单，学生不能自主加入课程。首次签到事务冻结名单，后续退课不会修改课次历史。课次支持一次签到、任意已发起的续签及一次签退；窗口不可重叠，签退后不能再开启阶段。投屏二维码仅所属教师可取，按 5 秒 slot 更新，提前关闭后立即失效，响应禁止缓存。名单、越权、快照及阶段转换已通过真实 HTTP 进程烟测。

正常签到三因素全通过自动成功，恰好两因素通过仅获得申请资格；只有显式提交申请后才进入人工审核。续签和签退只需定位、人脸，均不开放部分审核例外。签到窗口关闭后的迟到候选截止于课次预定开始时间 +15 分钟，仍须教师批准。汇总要求所有实际发起的阶段成功；批准请假覆盖最终结果，但不删除阶段证据。真实 HTTP 已验证登记、三因素签到、部分审核、续签、签退、迟到、请假及附件越权；自然结束时的在途提交被拒绝，SQLite 写锁等待后返回 `503 busy`。

迁移后 `cargo test --workspace` 已通过 25 个回归用例；独立默认 feature 的核心 QR 回归及 WASM target 构建检查也通过。真实后端加载两个 Rust 人脸实例后 `/readyz` 返回 200；缺模型时以 `Models` 错误退出。此前的完整 HTTP 验收确认：重启后既有 cookie、课程、审核与最终汇总保留，T1 登记、三因素签到、无二维码续签/签退及 `present/true` 汇总通过；未知/重复字段、非法 MIME/图像尺寸、单图/文本/附件大小、跨学生 nonce、迟到及续签禁传二维码、分页边界均按契约处理；合法 2.7 MiB multipart 不受 Axum 默认 2 MiB 上限误伤。
