# AI Studio 파일 포맷 스펙 v1

## 설계 원칙

1. **프로젝트는 폴더다.** 단일 바이너리(zip) 파일이 아니라 일반 디렉터리. `git diff`가 동작하고, `grep`이 동작하고, RAG 인덱서가 파일 단위로 청킹할 수 있다.
2. **내용은 Markdown, 배치는 JSON.** 사람이 읽는 텍스트/의미는 `.md`, 좌표·스타일·계산은 `.json`. 둘은 **블록 ID**로 연결된다.
3. **Markdown은 손으로 고칠 수 있다.** md만 편집해도 앱이 열린다 (없는 배치는 자동 레이아웃으로 채움).
4. **JSON은 진실의 원천(source of truth)이고, md는 투영(projection)이다.** 단 텍스트 내용에 관해서는 md가 원천이다. 겹치지 않게 역할을 나눈다.
5. **`AI.md`는 자동 생성 다이제스트.** 프로젝트 전체를 한 파일로 요약해 LLM 컨텍스트에 그대로 넣을 수 있게 한다.

## 공통 구조

```
<Title>.aideck/          # 프레젠테이션
<Title>.aidoc/           # 문서
<Title>.aigrid/          # 스프레드시트
├── manifest.json        # 문서 타입, 순서, 테마, 메타데이터
├── AI.md                # ⚙️ 자동 생성 — RAG/LLM용 전체 다이제스트
├── slides|content|sheets/
│   ├── 01-title.md          # 내용 (Markdown)
│   └── 01-title.layout.json # 배치/스타일 (JSON)
└── assets/              # 이미지 등 바이너리
```

### manifest.json (공통 필드)

```json
{
  "format": "ai-studio/deck",
  "formatVersion": 1,
  "id": "prj_k3n8x1",
  "title": "Q3 Business Review",
  "created": "2026-09-01T04:30:00.000Z",
  "modified": "2026-09-01T05:12:44.000Z",
  "theme": { "name": "aurora", "accent": "#4f46e5", "font": "Inter" }
}
```

타입별 추가 필드: `slides[]` / `sections[]` / `sheets[]` — 각 항목은 `{ id, name, md, json }` 경로 쌍.

---

## 1. Deck (`.aideck`) — 프레젠테이션

핵심: **슬라이드 1장 = md 1개 + layout.json 1개.**

### `slides/01-title.md`

```markdown
---
id: s1
title: 표지
layout: title
notes: |
  인사하고 3분 안에 아젠다로 넘어간다.
---

<!-- block:b1 -->
# 2026 3분기 사업 리뷰

<!-- block:b2 -->
매출 성장과 신규 시장 진입 결과 보고

<!-- block:b3 -->
![팀 사진](../assets/team.png)
```

* 블록 구분자는 `<!-- block:ID -->`. HTML 주석이므로 어떤 Markdown 렌더러에서도 안전하게 무시된다.
* 블록 안의 내용은 **평범한 Markdown**이다 (제목, 리스트, 표, 이미지, 코드펜스).
* 프론트매터의 `notes`는 발표자 노트 → LLM이 화자 의도를 읽을 수 있다.
* `layout`은 의미적 레이아웃 이름 (`title`, `title-content`, `two-column`, `blank`).

### `slides/01-title.layout.json`

```json
{
  "id": "s1",
  "canvas": { "w": 1280, "h": 720, "bg": "#ffffff" },
  "blocks": {
    "b1": { "x": 96, "y": 220, "w": 1088, "h": 120, "z": 1, "kind": "text",
            "style": { "fontSize": 56, "weight": 700, "align": "center", "color": "#111827" } },
    "b2": { "x": 96, "y": 360, "w": 1088, "h": 60, "z": 2, "kind": "text",
            "style": { "fontSize": 22, "align": "center", "color": "#6b7280" } },
    "b3": { "x": 440, "y": 440, "w": 400, "h": 220, "z": 3, "kind": "image",
            "style": { "fit": "cover", "radius": 12 } }
  }
}
```

`kind`: `text` | `image` | `shape` | `table` | `chart`.
좌표계는 캔버스 픽셀 기준 (기본 1280×720, 16:9).

---

## 2. Doc (`.aidoc`) — 문서

핵심: **섹션 1개 = md 1개 + meta.json 1개.** 흐름(flow) 레이아웃이므로 x/y 좌표가 없고, 대신 **문서 구조와 스타일 오버라이드**를 JSON이 갖는다.

### `content/01-intro.md`

깨끗한 순수 Markdown이다. 기본 스타일을 쓰는 문단에는 주석조차 없다.

```markdown
---
id: sec1
name: 서론
---

# 서론

이 문서는 AI Studio의 저장 포맷을 설명한다.

<!-- block:p7 -->
> 인용문처럼 특별한 서식이 붙은 문단만 ID 주석을 갖는다.

## 배경

- 기존 오피스 포맷은 zip + XML이다
- LLM이 직접 읽기 어렵다
```

* **ID 주석은 비-기본 서식을 가진 블록에만 붙는다.** 덕분에 md가 지저분해지지 않는다.

### `content/01-intro.meta.json`

```json
{
  "id": "sec1",
  "page": { "size": "A4", "margin": { "top": 72, "right": 72, "bottom": 72, "left": 72 } },
  "blocks": {
    "p7": { "align": "center", "indent": 36, "spacing": { "before": 12, "after": 12 },
            "style": { "italic": true, "color": "#4f46e5" } }
  },
  "outline": [
    { "level": 1, "text": "서론", "anchor": "서론" },
    { "level": 2, "text": "배경", "anchor": "배경" }
  ],
  "stats": { "words": 42, "chars": 231 }
}
```

---

## 3. Grid (`.aigrid`) — 스프레드시트

핵심: **JSON이 계산의 원천, md는 사람·AI가 읽는 투영.**
md에는 **계산된 값**이 A/B/C 열 머리글과 1/2/3 행 번호와 함께 들어간다 → LLM이 "B5" 같은 셀 주소로 추론할 수 있다.

### `sheets/sales.md` (자동 생성 투영)

```markdown
---
id: sh1
name: 매출
dims: { rows: 6, cols: 4 }
---

## 매출

|   | A | B | C | D |
|---|---|---|---|---|
| **1** | 지역 | 1분기 | 2분기 | 합계 |
| **2** | 동부 | 1,200 | 1,350 | 2,550 |
| **3** | 서부 | 980 | 1,120 | 2,100 |
| **4** | **총계** | **2,180** | **2,470** | **4,650** |

### 수식

- `D2` = `=B2+C2` → 2550
- `D3` = `=B3+C3` → 2100
- `B4` = `=SUM(B2:B3)` → 2180
- `C4` = `=SUM(C2:C3)` → 2470
- `D4` = `=SUM(D2:D3)` → 4650

### 이름 있는 범위

- `매출데이터` → `A1:D4`
```

### `sheets/sales.cells.json` (원천)

```json
{
  "id": "sh1",
  "name": "매출",
  "dims": { "rows": 200, "cols": 26 },
  "frozen": { "rows": 1, "cols": 1 },
  "colWidths": { "A": 120 },
  "rowHeights": {},
  "cells": {
    "A1": { "v": "지역", "t": "s", "style": { "bold": true, "bg": "#f3f4f6" } },
    "B2": { "v": 1200, "t": "n", "fmt": "#,##0" },
    "D2": { "f": "=B2+C2", "v": 2550, "t": "n", "fmt": "#,##0" },
    "B4": { "f": "=SUM(B2:B3)", "v": 2180, "t": "n", "style": { "bold": true } }
  },
  "merges": [],
  "names": { "매출데이터": "A1:D4" }
}
```

* `f` = 수식, `v` = 마지막으로 계산된 값(캐시), `t` = 타입(`n`/`s`/`b`/`d`/`e`), `fmt` = 표시 형식.
* 수식 캐시 `v`가 있으므로 **계산 엔진 없이도** md 투영과 RAG가 정확한 값을 본다.

---

## 4. `AI.md` — RAG 다이제스트

저장할 때마다 자동 재생성된다. 프로젝트 전체를 하나의 마크다운으로 평탄화하며, LLM이 알아야 할 구조 정보(슬라이드 순서, 블록 위치의 의미, 수식)를 자연어로 붙인다.

```markdown
# Q3 Business Review
> AI Studio Deck · 슬라이드 3장 · 최종 수정 2026-09-01

## 슬라이드 1 — 표지 (layout: title)
발표자 노트: 인사하고 3분 안에 아젠다로 넘어간다.

### 2026 3분기 사업 리뷰
_위치: 상단 중앙_

매출 성장과 신규 시장 진입 결과 보고
_위치: 중앙_
```

위치를 픽셀이 아니라 **`상단 중앙`, `좌측 하단` 같은 자연어**로 번역하는 것이 핵심이다. LLM은 좌표보다 이 표현을 훨씬 잘 이해한다.

---

## 왜 이게 AI에게 유리한가

| | OOXML (.pptx/.docx/.xlsx) | AI Studio |
|---|---|---|
| 컨테이너 | zip + 수십 개 XML 파트 | 일반 폴더 |
| 텍스트 추출 | XML 파싱 필요, 런(run) 단위로 조각남 | md 그대로 읽으면 끝 |
| 위치 정보 | EMU 단위 좌표, 도형 트리 | JSON + 자연어 번역 |
| 청킹 단위 | 파일 전체 | 슬라이드/섹션/시트 = 파일 1개 |
| 버전 관리 | 바이너리 diff | 텍스트 diff |
| 수식 | 계산 결과가 별도 캐시 파트에 | `f`와 `v`가 한 객체에 |
