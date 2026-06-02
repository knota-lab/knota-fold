# Knota Fold

> 基于 [Loco-rs](https://loco.rs)（Axum + SeaORM）的后端开箱即用脚手架，内置多租户、RBAC 权限、国际化、文件管理等企业级能力。

## 技术栈

**后端**
- Rust · [Loco-rs](https://loco.rs) 0.16 · Axum 0.8 · SeaORM 1.1
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

### 本地开发

**1. 启动基础设施**

```bash
# MinIO（对象存储）+ Qdrant（向量数据库）+ Mailpit（邮件测试），默认启动
docker compose -f docker/docker-compose.yml up -d

# 如需 PostgreSQL（默认使用 SQLite，无需启动）
docker compose -f docker/docker-compose.yml --profile postgresql up -d

# 如需 Redis（默认使用 InMem 缓存，无需启动）
docker compose -f docker/docker-compose.yml --profile redis up -d
```

MinIO 管理面板：http://localhost:9001（`minioadmin` / `minioadmin`）
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
cargo loco task bootstrap_admin email:admin@example.com password:admin123
```

> 首次启动后执行，幂等（邮箱已存在则跳过）。可选参数：`name:显示名称`、`tenant_id:租户UUID`。

### Staging / 线上开发环境

构建后端 + 前端镜像，启动完整服务栈。Caddy 作为统一入口（安全头 + 限流 + API 反代 + SSL）：

```bash
# 启动完整 staging 环境（Caddy + 后端 + PG + Redis + MinIO + Mailpit）
docker compose -f docker/docker-compose.staging.yml up -d --build

# 同时启动 EasyTier（虚拟局域网，方便团队远程访问）
docker compose -f docker/docker-compose.staging.yml --profile easytier up -d --build
```

自定义配置（复制并修改）：

```bash
cp docker/.env.example docker/.env.staging
# 编辑 .env.staging，修改 JWT_SECRET、POSTGRES_PASSWORD 等
```

首次启动后创建管理员：

```bash
docker compose -f docker/docker-compose.staging.yml exec backend \
  knota_fold-cli task bootstrap_admin email:admin@example.com password:your_password
```

也可通过环境变量传入（适合 CI/CD）：

```bash
docker compose -f docker/docker-compose.staging.yml exec -e BOOTSTRAP_ADMIN_EMAIL=admin@example.com -e BOOTSTRAP_ADMIN_PASSWORD=your_password backend \
  knota_fold-cli task bootstrap_admin
```

### 生产部署

```bash
# 复制并填写生产环境配置
cp docker/.env.example docker/.env.production
# 编辑 .env.production，所有标记为 required 的变量必须填写

# 启动（Caddy + PG + Redis + MinIO + 后端）
docker compose -f docker/docker-compose.prod.yml up -d --build
```

首次启动后创建管理员：

```bash
docker compose -f docker/docker-compose.prod.yml exec backend \
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
| 本地基础设施 | `docker compose -f docker/docker-compose.yml up -d` |
| 本地 + PG | 加 `--profile postgresql` |
| 本地 + PG + Redis | 加 `--profile postgresql --profile redis` |
| Staging 全栈 | `docker compose -f docker/docker-compose.staging.yml up -d --build` |
| Staging + EasyTier | 加 `--profile easytier` |
| 生产部署 | `docker compose -f docker/docker-compose.prod.yml up -d --build` |
| 查看日志 | `docker compose -f docker/docker-compose.staging.yml logs -f backend` |
| 停止 | `docker compose -f docker/docker-compose.staging.yml down` |
| 清理数据 | `docker compose -f docker/docker-compose.staging.yml down -v` |

## EasyTier（虚拟局域网）

通过 EasyTier 将云上服务器和开发团队组成虚拟局域网，服务器只需开放一个端口：

1. 编辑 `docker/easytier/easytier-server.toml`，设置 `network_secret`
2. 云上启动时加 `--profile easytier`
3. 团队成员使用 `docker/easytier/easytier-client.toml` 连接

详见 `docker/easytier/` 目录下的配置模板。

## 项目结构

```
knota-fold/                       # 后端
├── src/
│   ├── controllers/              # API 控制器
│   ├── services/                 # 业务逻辑层
│   ├── models/                   # SeaORM 数据模型
│   ├── views/                    # 请求/响应 DTO
│   ├── middleware/                # 中间件（Casbin、RBAC、Tracing）
│   ├── initializers/             # 应用初始化器（Casbin、S3、OpenAPI）
│   ├── tasks/                    # 后台定时任务
│   ├── workers/                  # 异步 Worker
│   ├── mailers/                  # 邮件模板
│   ├── fixtures/                 # 种子数据
│   └── config/                   # 配置模块
├── migration/                    # 数据库迁移
├── config/                       # 环境配置文件（development / test / production）
├── docker/                       # Docker 配置
│   ├── Dockerfile                # 后端镜像（多阶段构建）
│   ├── docker-compose.yml        # 本地开发（依赖服务）
│   ├── docker-compose.staging.yml  # Staging（全栈构建）
│   ├── docker-compose.prod.yml   # 生产部署
│   ├── easytier/                 # EasyTier 配置模板
│   └── .env.example              # 环境变量文档
└── tests/                        # 集成测试

knota-studio/                     # 前端管理面板
├── src/
│   ├── pages/                    # 页面组件
│   ├── api/                      # API 客户端
│   ├── components/               # 通用组件（ProTable、DataTable、Form）
│   ├── stores/                   # 状态管理（Auth、Agent）
│   ├── i18n/                     # 国际化运行时
│   ├── layout/                   # 布局组件（侧边栏、面包屑）
│   ├── lib/                      # 工具库（Agent 集成、Iconify）
│   └── types/                    # TypeScript 类型定义
├── Caddyfile                     # Caddy 配置（安全头 + 限流 + 反代）
├── Dockerfile                    # 前端镜像（多阶段构建：pnpm → Caddy）
└── public/                       # 静态资源
```

## 测试

```bash
# 运行全部 API Key 测试（推荐，避免 mmap 问题）
cargo test --test mod api_keys

# 运行全部测试（注意：全量编译可能触发 mmap os error 1455）
cargo test --test mod
```
