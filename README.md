# Knota Fold

> 基于 [Loco-rs](https://loco.rs)（Axum + SeaORM）的后端开箱即用脚手架，内置多租户、RBAC 权限、国际化、文件管理等企业级能力。

## 技术栈

**后端**
- Rust · [Loco-rs](https://loco.rs) 1.3 · Axum 0.8 · SeaORM 2.0
- Casbin RBAC · JWT + API Key 双路认证
- SQLite（开发）/ PostgreSQL（生产）· Redis 缓存
- S3 / MinIO 对象存储 · 后台任务调度 · Qdrant 向量数据库

**前端**
- React 19 · TypeScript 6 · TanStack Table/Form · Tailwind CSS 4
- Rsbuild · Biome · Vitest · Iconify (Lucide)
- 运行时 i18n（惰性加载 + ETag 缓存 + CI 提取）

**反向代理**
- Caddy（自动 SSL + 安全头 + 限流 + SPA 静态文件）

## 功能一览

| 模块 | 说明 |
|------|------|
| 认证授权 | JWT 登录、图片验证码、RBAC（Casbin）、角色模板、API Key |
| 多租户 | 租户隔离、租户级配置覆盖（菜单/字典/i18n/系统配置） |
| 用户管理 | CRUD、角色分配、状态控制、密码重置、超级管理员 |
| 菜单管理 | 系统菜单树、租户覆盖、角色菜单绑定 |
| 字典管理 | 字典类型/项 CRUD、树形结构、租户覆盖 |
| 文件管理 | S3 存储、分片上传、秒传（BLAKE3 去重）、文件引用 |
| 国际化 | DB 翻译、命名空间、Bundle 缓存、CI 自动提取、租户覆盖 |
| 审计日志 | 全变更审计、快照 + Diff、租户隔离 |
| 任务调度 | Cron 调度、执行追踪、重试、租户授权 |
| 配置中心 | 全局/租户配置、分层解析 |
| API Key | Key 管理、兑换令牌、自助兑换 |
| 知识库 | Qdrant 向量检索、Agent 对话、SSE 流式响应 |
| OpenAPI | Swagger UI / Redoc / Scalar 三套文档 |

## 快速开始

### 环境要求

- Rust 1.85+（`rustup update stable`）
- Node.js 22+ / pnpm 10+
- Docker & Docker Compose（用于本地基础设施和部署）

### 推荐：创建完整项目

如果是创建新的业务项目，推荐使用独立 CLI [`knota-dev`](https://github.com/knota-lab/knota-dev)，它会下载 `knota-fold` 和 `knota-studio`，并基于 `knota-fold/docker/.env.example` 生成随机本地密钥：

```bash
cargo install knota-dev
knota-dev new my-knota-app
cd my-knota-app
cp knota-fold/docker/.env.example knota-fold/docker/.env
docker compose --env-file knota-fold/docker/.env -f knota-fold/docker/docker-compose.yml up -d
```

`knota-dev` 只负责项目脚手架，不接管运行时进程管理。

### 本地开发

如果你只在本仓库内开发后端，先准备 Docker 环境变量：

```bash
cp docker/.env.example docker/.env
# 编辑 docker/.env，至少确认 POSTGRES_PASSWORD、REDIS_PASSWORD、S3_ACCESS_KEY、S3_SECRET_KEY、JWT_SECRET 等值不是默认占位符。
```

**1. 启动基础设施**

```bash
# MinIO（对象存储）+ Qdrant（向量数据库）+ Mailpit（邮件测试），默认启动
docker compose --env-file docker/.env -f docker/docker-compose.yml up -d

# 如需 PostgreSQL（默认使用 SQLite，无需启动）
docker compose --env-file docker/.env -f docker/docker-compose.yml --profile postgresql up -d

# 如需 Redis（默认使用 InMem 缓存，无需启动）
docker compose --env-file docker/.env -f docker/docker-compose.yml --profile redis up -d
```

MinIO 管理面板：http://localhost:9001（账号密码来自 `docker/.env` 的 `S3_ACCESS_KEY` / `S3_SECRET_KEY`）
Mailpit 邮件面板：http://localhost:8025

**2. 启动后端**

```bash
cargo loco start --all
```

后端运行在 http://localhost:5150，API 文档在 http://localhost:5150/swagger

`--all` 启用后台 Worker 和定时任务调度。

**3. 启动前端**

```bash
cd ../knota-studio
pnpm install
pnpm run dev
```

前端运行在 http://localhost:3000，通过 Rsbuild proxy 将 `/api/` 代理到后端 5150 端口。

**4. 创建管理员账号**

```bash
cargo loco task bootstrap_admin email:admin@example.com password:<your-strong-password>
```

> 系统不会自动创建默认管理员账号。首次启动后必须显式执行该命令；命令幂等（邮箱已存在则跳过）。可选参数：`name:显示名称`、`tenant_id:租户UUID`。

### Staging / 线上开发环境

构建后端 + 前端镜像，启动完整服务栈。Caddy 作为统一入口（安全头 + 限流 + API 反代 + SSL）：

```bash
# 启动完整 staging 环境（Caddy + 后端 + PG + Redis + MinIO + Mailpit）
docker compose --env-file docker/.env.staging -f docker/docker-compose.staging.yml up -d --build

# 同时启动 EasyTier（虚拟局域网，方便团队远程访问）
docker compose --env-file docker/.env.staging -f docker/docker-compose.staging.yml --profile easytier up -d --build
```

自定义配置（复制并修改）：

```bash
cp docker/.env.example docker/.env.staging
# 编辑 .env.staging，修改 JWT_SECRET、POSTGRES_PASSWORD 等
```

首次启动后创建管理员：

```bash
docker compose --env-file docker/.env.staging -f docker/docker-compose.staging.yml exec backend \
  knota_fold-cli task bootstrap_admin email:admin@example.com password:your_password
```

也可通过环境变量传入（适合 CI/CD）：

```bash
docker compose --env-file docker/.env.staging -f docker/docker-compose.staging.yml exec -e BOOTSTRAP_ADMIN_EMAIL=admin@example.com -e BOOTSTRAP_ADMIN_PASSWORD=your_password backend \
  knota_fold-cli task bootstrap_admin
```

### 生产部署

```bash
# 复制并填写生产环境配置
cp docker/.env.example docker/.env.production
# 编辑 .env.production，所有标记为 required 的变量必须填写

# 启动（Caddy + PG + Redis + MinIO + 后端）
docker compose --env-file docker/.env.production -f docker/docker-compose.prod.yml up -d --build
```

首次启动后创建管理员：

```bash
docker compose --env-file docker/.env.production -f docker/docker-compose.prod.yml exec backend \
  knota_fold-cli task bootstrap_admin email:admin@example.com password:your_password
```

生产环境特性：
- Caddy 提供统一入口：安全头 + 限流 + SSL 自动续期 + SPA 静态文件
- PostgreSQL / Redis / MinIO **不暴露端口**到宿主机
- 后端仅对内暴露（`expose`），外部请求经 Caddy 反代
- 关键变量（`JWT_SECRET`、`DATABASE_URL` 等）未配置时启动报错
- 应用日志持久化（`app_logs` volume）
- 设置 `DOMAIN` 环境变量后 Caddy 自动申请 Let's Encrypt 证书

### Docker Compose 命令速查

| 场景 | 命令 |
|------|------|
| 本地基础设施 | `docker compose --env-file docker/.env -f docker/docker-compose.yml up -d` |
| 本地 + PG | 加 `--profile postgresql` |
| 本地 + PG + Redis | 加 `--profile postgresql --profile redis` |
| Staging 全栈 | `docker compose --env-file docker/.env.staging -f docker/docker-compose.staging.yml up -d --build` |
| Staging + EasyTier | 加 `--profile easytier` |
| 生产部署 | `docker compose --env-file docker/.env.production -f docker/docker-compose.prod.yml up -d --build` |
| 查看日志 | `docker compose --env-file docker/.env.staging -f docker/docker-compose.staging.yml logs -f backend` |
| 停止 | `docker compose --env-file docker/.env.staging -f docker/docker-compose.staging.yml down` |
| 清理数据 | `docker compose --env-file docker/.env.staging -f docker/docker-compose.staging.yml down -v` |

## EasyTier（虚拟局域网）

通过 EasyTier 将云上服务器和开发团队组成虚拟局域网，服务器只需开放一个端口：

1. 编辑 `docker/easytier/easytier-server.toml`，设置 `network_secret`
2. 云上启动时加 `--profile easytier`
3. 团队成员使用 `docker/easytier/easytier-client.toml` 连接

详见 `docker/easytier/` 目录下的配置模板。

## API Key 认证

API Key 有两种用法：

1. 直接作为 Bearer token（兼容旧客户端）：

```http
Authorization: Bearer sk_live_xxxxxxxx
```

2. 先兑换短期 JWT，后续请求使用 JWT（推荐）：

```bash
curl -X POST https://your-host/api/auth/api-key/exchange \
  -H "Authorization: Bearer sk_live_xxxxxxxx"
```

响应中的 `token` 默认 5 分钟有效，之后使用：

```http
Authorization: Bearer <短期JWT>
```

兑换后的 JWT 带有 `auth_type=api_key`、`api_key_id`、`tenant_id`、`role` 等签名 claims，鉴权走 `apikey:<id>` 的 Casbin 策略，不再每请求查询 users/tenants/roles。API Key 被撤销后，已签发的 JWT 最多在 TTL（默认 300 秒）后失效；需要立即失效时请缩短 TTL 或等待后续 epoch 支持。

## 项目结构

```text
knota-fold/
├── src/
│   ├── modules/                  # 按业务域组织（controller / service / models / views）
│   │   ├── auth/                 # 登录、验证码、登录锁定、JWT 缓存
│   │   ├── users/
│   │   ├── roles/                # 角色、角色模板
│   │   ├── permissions/
│   │   ├── tenants/
│   │   ├── menus/                # 菜单、租户菜单覆盖
│   │   ├── dicts/
│   │   ├── files/                # 文件、分片上传、引用、文件审计
│   │   ├── i18n/
│   │   ├── audit_logs/
│   │   ├── sys_configs/
│   │   ├── api_keys/             # API Key + 兑换令牌 + 短期 JWT
│   │   ├── app_logs/             # 请求日志、日志落库
│   │   ├── workers/              # 调度管理 API
│   │   ├── notification/
│   │   └── knowledge_base/       # 文档、检索、QA、向量
│   ├── models/_entities/         # SeaORM 生成实体（共享，不随域移动）
│   ├── services/                 # 跨域基础设施：Casbin、resource_types
│   ├── views/                    # 共享错误、分页
│   ├── middleware/               # JWT/API Key 鉴权、Tracing、错误处理
│   ├── extractors/               # TenantContext 等
│   ├── initializers/             # 应用初始化器
│   ├── utils/                    # 错误、ID、时间等
│   ├── config/                   # 配置结构
│   ├── fixtures/                 # 种子数据
│   ├── mailers/                  # 邮件模板
│   ├── tasks/                    # Loco 定时任务
│   └── workers/                  # Loco 后台 Worker
├── migration/                    # 数据库迁移
├── config/                       # 环境配置（development / test / production）
├── docker/                       # Docker 配置
└── tests/                        # 集成测试
```

业务代码统一使用 `crate::modules::<domain>::...`。`src/models/` 只保留 SeaORM `_entities`，`src/services/` 只保留 Casbin 与 resource_types，`src/views/` 只保留共享错误与分页。

## 测试

```bash
# 运行全部 API Key 测试（推荐，避免 mmap 问题）
cargo test --test mod api_keys

# 运行全部测试（注意：全量编译可能触发 mmap os error 1455）
cargo test --test mod
```
