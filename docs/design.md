# xlpod — 프로젝트 설계서 (v0)

> Excel/xlwings의 설치 고통을 없애는 Tauri 트레이 런처 + xlwings Lite(Pyodide)에서 로컬 PC를 안전하게 사용할 수 있게 하는 PyPI 클라이언트.

---

## 0. 결정 사항 요약

| 질문 | 답 |
|---|---|
| dartlab과 같은 레포? | **분리.** 사용자/릴리즈 주기/언어 스택/라이선스 모두 다름 |
| 레포명 | **`xlpod`** (대안: `xlpilot`, `excelbridge`) |
| PyPI 런칭 필요한가 | **필요. 단 런처가 아니라 "Pyodide bridge 클라이언트"가 PyPI에 올라감** |
| 배포 채널 | GitHub Releases(런처 exe) + PyPI(`xlpod` wheel) + winget(선택) |
| 1차 OS | **Windows만.** macOS는 Phase 5 |
| 라이선스 | **Apache-2.0** |
| Phase 0 go/no-go | **🟢 GREEN (2026-04-07)** — Lite origin = `https://addin.xlwings.org`, CSP `connect-src https: wss:`, mkcert 경로 동작 확인. 상세: [phase0-report.md](phase0-report.md) |

### 레포명 — `xlpod` 추천 이유

- 짧고(8자) 발음 쉬움, `pip install xlpod` 깔끔
- "xlwings"를 이름에 박으면 상표/혼동 이슈 (Felix Zumstein이 PRO 사업 중)
- "bridge"가 정확히 우리가 하는 일 (Pyodide ↔ 로컬, Excel ↔ Python)
- GitHub: `eddmpython/xlpod`
- **PyPI + GitHub + 도메인 사전 점유 확인 필수.** 점유돼 있으면 차순위 `xlpilot`

---

## 1. 왜 별도 레포인가 — 7가지 근거

1. **언어 스택**: dartlab=Python+Svelte, xlpod=**Rust(Tauri)+Python**
2. **사용자**: dartlab=한국 주식 분석가, xlpod=Excel+Python 사용자(글로벌). 겹침 거의 0
3. **릴리즈 주기**: dartlab=patch 자주, xlpod=보안 패치 + xlwings upstream 변동
4. **라이선스 격리**: pywin32(PSF) + Tauri(MIT/Apache) + 임베디드 Python 재배포(PSF)
5. **코드 서명 인증서**: 런처 exe 배포에 EV/OV 필요, dartlab은 wheel만이라 불필요
6. **이슈 트래커**: "xlwings 안 깔려요" 이슈가 dartlab에 섞이면 시그널 손실
7. **언어**: dartlab=한국어 우선, xlpod=영어 우선 (xlwings 글로벌 기반)

---

## 2. 패키지 구조 — 런처는 PyPI가 아니다

### 2.1 산출물 매트릭스

| 산출물 | 채널 | 언어 | 대상 | 이유 |
|---|---|---|---|---|
| `xlpod.exe` (런처) | **GitHub Releases + winget** | Rust (Tauri) | 일반 Excel 사용자 | exe는 PyPI 못 올림. 코드서명 필요 |
| `xlpod` (Pyodide 클라이언트) | **PyPI** (pure python wheel) | Python | xlwings Lite 사용자 | `micropip.install("xlpod")` 필수 |
| `xlpod-cli` (선택) | PyPI | Python | 파워유저, CI | 런처 없이 헤드리스 |

**런처를 PyPI에 올리려는 유혹 금지** — Tauri 바이너리는 PyPI policy 위반, 200MB 인스톨러를 wheel에 못 넣음.

### 2.2 PyPI `xlpod` 패키지의 정체

**pure-python 얇은 클라이언트**:

```python
# xlpod/__init__.py
from .client import connect, fs, run, excel
from .errors import LauncherNotRunning, PermissionDenied
```

- 의존성: Pyodide의 `pyodide.http` + CPython의 `httpx` 어댑터
- **Pyodide와 CPython 양쪽에서 import 가능** (Lite + 일반 xlwings 양쪽 사용)
- wheel: `py3-none-any` (universal), 크기 < 50KB

PyPI에 있어야 하는 이유: xlwings Lite의 `requirements.txt`에 `xlpod` 한 줄 적으면 micropip이 PyPI에서 받아옴. 다른 경로 없음.

### 2.3 모노레포 구조

```
xlpod/
├── launcher/                  # Rust + Tauri v2
│   ├── src-tauri/
│   │   ├── src/
│   │   │   ├── main.rs
│   │   │   ├── server/        # axum HTTPS
│   │   │   │   ├── routes/    # fs, run, excel, health
│   │   │   │   ├── auth.rs    # 토큰 + origin
│   │   │   │   └── tls.rs     # rustls + 자체 CA
│   │   │   ├── installer/     # 임베디드 Python, xlwings
│   │   │   ├── excel/         # Office 감지, 레지스트리, addin
│   │   │   ├── updater/       # PyPI 폴링, 셀프업데이트
│   │   │   └── tray.rs
│   │   ├── Cargo.toml
│   │   └── tauri.conf.json
│   └── ui/                    # Svelte (트레이, 동의 다이얼로그)
├── client/                    # PyPI: xlpod
│   ├── xlpod/
│   │   ├── __init__.py
│   │   ├── client.py          # httpx + pyodide.http 어댑터
│   │   ├── fs.py
│   │   ├── run.py
│   │   ├── excel.py
│   │   └── _proto.py          # 프로토콜 상수
│   ├── tests/
│   └── pyproject.toml
├── proto/
│   └── xlpod.openapi.yaml  # 단일 진실
├── docs/
├── examples/
│   ├── lite-hello/
│   └── desktop-hello/
├── .github/workflows/
│   ├── launcher-release.yml   # tag v* → 서명 + Releases
│   ├── client-release.yml     # tag client-v* → PyPI trusted publishing
│   └── ci.yml
├── LICENSE                    # Apache-2.0
├── NOTICE
└── README.md
```

### 2.4 버전 정책 — 런처와 클라이언트 독립 SemVer

- 런처: `v1.2.3`
- 클라이언트: `client-v0.4.1`
- **프로토콜 버전 별도**. 헤더 `X-XLPod-Proto: 1`로 협상

호환성 매트릭스를 docs에 표로 유지.

---

## 3. 핵심 기술 결정

### 3.1 Tauri v2 선택

| 후보 | 결정 | 이유 |
|---|---|---|
| **Tauri v2** | ✅ | ~10MB, Rust 메모리 안전, sidecar/updater 성숙 |
| Electron | ❌ | 150MB+, Chromium 패치 책임 |
| .NET WinForms | ❌ | MS 의존 |
| Wails (Go) | ❌ | 성숙도 부족 |

### 3.2 임베디드 Python

**python.org embeddable distribution**:
- Python **3.12.x** 핀
- `%LOCALAPPDATA%\xlpod\runtime\py312\`
- 비트는 사용자 Excel 비트와 일치 (런처가 감지)
- 첫 실행 시 SHA256 검증 후 다운로드 (또는 인스톨러 번들)
- pip 부트스트랩 → `python._pth` 수정

**기각**: PyOxidizer (pywin32 마찰), conda (600MB)

### 3.3 로컬 서버

| 항목 | 결정 |
|---|---|
| 프레임워크 | **axum** (tokio + rustls) |
| TLS | **rustls + 자체 CA** (mkcert 방식) |
| 포트 | 7421 고정 + 7422-7430 폴백 |
| 바인딩 | `127.0.0.1` + `::1`만 |
| 프로토콜 | REST + WebSocket (`/ws`) |

### 3.4 인증서 신뢰 — 가장 까다로운 부분

Pyodide(WebView2)에서 `fetch('https://127.0.0.1:7421')` 하려면 인증서 신뢰 필요.

| 방법 | 평가 |
|---|---|
| **로컬 CA + Windows root store 등록** (mkcert) | 동작 확실, UAC 1회 |
| Let's Encrypt for `127.0.0.1` | 불가능 |
| HTTP만 | 불가능 (Lite는 HTTPS origin) |

→ **로컬 CA + root store 등록이 유일한 실용 경로.**

### 3.5 보안 모델 — Defense in Depth

10개 레이어:

1. **Loopback only** — 컴파일 타임 상수, `0.0.0.0` 금지
2. **TLS 필수** — *모든* 엔드포인트. (Phase 0 측정 결과 Lite CSP가 `upgrade-insecure-requests` + `block-all-mixed-content` 를 강제하므로 평문 fallback 자체가 불가능 → 평문 코드 경로 삭제)
3. **Origin 화이트리스트** — `https://addin.xlwings.org` (Phase 0 실측 확정)
4. **세션 토큰** — 256bit random, 시작 시마다 새로 발급, `Bearer` 헤더 필수
5. **권한 스코프** — `fs:read`, `fs:write`, `run:python`, `excel:com`
6. **사용자 동의** — 민감 작업은 트레이 토스트 → 확인 → 토큰에 grant
7. **DNS rebinding 방어** — `Host` 헤더 검증
8. **감사 로그** — `audit.log` JSONL, UI에서 열람
9. **속도 제한** — 토큰별 100 req/s
10. **CORS** — preflight도 화이트리스트

> **Zoom 2019 사고**: origin 검증 + 토큰 둘 다 빠뜨려서 발생. 우리는 둘 다 한다.

### 3.6 Python 호출 모델

**장기 실행 Python worker + JSON-RPC over stdin/stdout**:
- 워크북당 1 worker, 유휴 60초 후 종료
- 단일 worker 800MB 초과 시 재시작 (dartlab BoundedCache 교훈)
- 매 요청 spawn(200ms) 대비 50ms

### 3.7 자동 업데이트

- **런처**: `tauri-plugin-updater` + GitHub Releases `latest.json` (Ed25519 서명)
- **xlwings/pywin32**: PyPI JSON 폴링 6시간마다, **Excel 미실행 감지 후** in-place 교체
- **xlpod client**: Lite의 `requirements.txt`가 PyPI 자동 — 프로토콜 버전으로 호환

---

## 4. API 표면 (proto v1)

```
GET  /health                      → {status, version, proto}
POST /auth/handshake              → {token, scopes, expires_in}

GET  /fs/read?path=...
POST /fs/write
GET  /fs/list?path=...
GET  /fs/stat?path=...

POST /run/python                  → {code, workbook?} → {stdout, stderr, result}
POST /run/script                  → {path, args}

POST /excel/open                  → {path}
GET  /excel/workbooks
POST /excel/range/read
POST /excel/range/write
POST /excel/macro

WS   /ws                          → 양방향 이벤트

GET  /launcher/version
POST /launcher/install            → xlwings 설치/업데이트
GET  /launcher/diagnose           → 환경 진단
```

Pyodide 클라이언트:

```python
import xlpod

xlpod.connect()
xlpod.fs.read("C:/data.csv")
xlpod.run("import pandas; ...")
xlpod.excel.range("Sheet1!A1:B10").value
```

---

## 5. 설치 자동화 시퀀스

`xlpod install`:

1. **환경 진단** — Excel 비트/버전, 기존 Python, addin 위치, 정책 차단
2. **런타임 설치** — embeddable Python 다운로드/검증/해제
3. **pip 부트스트랩** — get-pip.py
4. **xlwings 설치** — `pip install xlwings pywin32 openpyxl`
5. **pywin32 post-install** — COM 등록
6. **addin 설치** — `xlwings addin install`
7. **Trust 등록** — 레지스트리 (Office 버전 분기)
8. **Interpreter 주입** — `HKCU\Software\xlwings\Conf\Interpreter_Win`
9. **검증** — 가짜 워크북으로 동작 확인
10. **트레이 상주 시작** — autostart 등록

각 단계는 **idempotent + rollback 가능**해야 함.

---

## 6. 테스트 전략

| 레벨 | 도구 |
|---|---|
| 단위 (Rust) | `cargo test` |
| 단위 (Python client) | `pytest` |
| 통합 | `pytest` + 런처 sidecar |
| Pyodide 통합 | `pytest-pyodide` |
| Excel E2E | `pywinauto` 또는 수동 매트릭스 (Office 2019/2021/365, 32/64bit) |
| 보안 | `cargo-audit`, `bandit`, OWASP ZAP |

CI: GitHub Actions windows-latest. 실제 Excel은 self-hosted runner.

---

## 7. 라이선스 / 법무

- **본 프로젝트**: Apache-2.0
- **재배포**: Python(PSF), xlwings(BSD-3), pywin32(PSF), Tauri(MIT/Apache), rustls. 전부 호환
- **xlwings 상표**: 이름에 미사용. 출시 전 Felix Zumstein에게 사전 공지 권장
- **코드 서명**: DigiCert/Sectigo **EV ~$400/년** (SmartScreen 즉시 통과)
- **개인정보**: 텔레메트리 기본 OFF, opt-in only

---

## 8. 로드맵

### Phase 0 — 설계 확정 ✅ 완료 (2026-04-07)
- ~~`xlpod` 이름 점유 확인~~ (별도 체크)
- ~~OpenAPI proto v1~~ → Phase 1로 이월
- STRIDE 위협 모델 → [threat-model.md](threat-model.md)
- ✅ **xlwings Lite 실제 origin/CSP 측정** — GREEN, [phase0-report.md](phase0-report.md) 참조

### Phase 1 — 런처 MVP (2-3주)
- Tauri v2 셋업, 임베디드 Python, xlwings 자동 설치
- Interpreter 주입, Excel diagnose
- 트레이 + autostart
- **이 시점에 이미 사용자 가치 큼**

### Phase 2 — 로컬 서버 + Desktop 클라이언트 (2주)
- axum HTTPS, 자체 CA, 토큰/origin/consent
- `xlpod` PyPI 패키지 (CPython only 먼저)

### Phase 3 — Pyodide / xlwings Lite (2주)
- Pyodide 어댑터, micropip 설치 검증
- 실제 Lite 워크북 동작
- 보안 하드닝

### Phase 4 — 배포 (1주)
- 코드 서명 인증서, GitHub Actions 서명 워크플로
- winget manifest, 문서, 데모

### Phase 5 — macOS (평가)
- Excel for Mac은 sandbox 강함. AppleScript bridge 검토

---

## 9. 위험 등록부

| 위험 | 영향 | 완화 |
|---|---|---|
| ~~Lite origin/CSP가 fetch 차단~~ | ~~치명~~ | **해소됨 (Phase 0 GREEN, 2026-04-07).** `connect-src https: wss:` 확인 |
| WebView2가 자체 CA 신뢰 안 함 | 높음 | mkcert root store 등록 |
| Defender가 런처 격리 | 높음 | EV 서명 + 평판 + false positive 신고 |
| 회사 PC 정책 차단 | 중 | MSI + GPO 친화 문서 |
| xlwings PRO 충돌 | 중 | PRO 기능 미사용, 보완 위치 명확화 |
| Felix Zumstein 적대화 | 중 | 사전 공지, 이름 미사용, "더 쉽게"로 포지셔닝 |
| Pyodide 업데이트로 깨짐 | 중 | proto 협상, CI 매트릭스 |
| 토큰 유출 | 높음 | 매 시작 발급, scope 최소화, audit |
| 자체 CA 악용 | **치명** | 사용자 머신 only, 발급 도메인 화이트리스트 |

---

## 10. 액션 아이템

1. **`xlpod` 이름 점유 확인** — PyPI, GitHub, 도메인
2. 점유되면 차순위 (`xlpilot`)
3. GitHub 레포 생성 (private → public)
4. 본 설계서를 `docs/design.md`로 (이미 완료)
5. **Phase 0 결정적 실험**: 실제 xlwings Lite + DevTools로 origin/CSP 측정
6. 결과 가지고 Phase 1 착수

---

## 한 줄 결론

**별도 레포 `xlpod`. 런처는 GitHub Releases, Pyodide 클라이언트는 PyPI. 보안은 처음부터 origin+토큰+TLS+consent. Phase 0의 origin 측정이 go/no-go.**
