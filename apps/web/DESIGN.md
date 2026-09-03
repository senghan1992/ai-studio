---
name: AI Studio
description: Office처럼 쓰고, AI가 읽을 수 있게 저장하는 오피스 스위트의 디자인 시스템
colors:
  deck: "#c43e1c"
  doc: "#185abd"
  grid: "#107c41"
  ink: "#201f1e"
  ink-2: "#484644"
  ink-3: "#797775"
  line: "#e1dfdd"
  line-2: "#edebe9"
  surface: "#ffffff"
  surface-2: "#faf9f8"
  surface-3: "#f3f2f1"
  hover: "#f3f2f1"
  selected: "#edebe9"
  focus: "#0f6cbd"
  danger: "#a4262c"
typography:
  display:
    fontFamily: "Pretendard Variable, Pretendard, -apple-system, Apple SD Gothic Neo, Malgun Gothic, Noto Sans KR, system-ui, sans-serif"
    fontSize: "31px"
    fontWeight: 750
    letterSpacing: "-0.025em"
  headline:
    fontFamily: "Pretendard Variable, Pretendard, sans-serif"
    fontSize: "17px"
    fontWeight: 600
  title:
    fontFamily: "Pretendard Variable, Pretendard, sans-serif"
    fontSize: "12px"
    fontWeight: 600
    letterSpacing: "0.04em"
  body:
    fontFamily: "Pretendard Variable, Pretendard, sans-serif"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 1.5
  label:
    fontFamily: "Pretendard Variable, Pretendard, sans-serif"
    fontSize: "10.5px"
    fontWeight: 400
  mono:
    fontFamily: "Cascadia Mono, SF Mono, Menlo, Consolas, monospace"
    fontSize: "12px"
rounded:
  sm: "4px"
  md: "6px"
  lg: "8px"
  pill: "999px"
spacing:
  xs: "4px"
  sm: "8px"
  md: "12px"
  lg: "16px"
components:
  button-primary:
    backgroundColor: "{colors.doc}"
    textColor: "{colors.surface}"
    rounded: "{rounded.sm}"
    padding: "7px 16px"
  button:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    rounded: "{rounded.sm}"
    padding: "7px 16px"
  button-hover:
    backgroundColor: "{colors.hover}"
  input:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    rounded: "{rounded.sm}"
    height: "26px"
    padding: "0 6px"
  chip:
    backgroundColor: "{colors.surface-3}"
    textColor: "{colors.ink-2}"
    rounded: "{rounded.pill}"
    padding: "3px 8px"
  card:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    rounded: "{rounded.lg}"
    padding: "18px"
---

# Design System: AI Studio

## Overview

**Creative North Star: "낯익은 책상 (The Familiar Desk)"**

리본은 그 자리에, 단축키는 그 손가락에. 이 시스템의 야심은 새로워 보이는 것이 아니라
**사용자가 "배울 게 없네"라고 느끼는 것**이다. 20년 된 Office의 근육 기억 — 리본 탭의 순서,
그룹의 이름, 도형 갤러리의 서랍, 우클릭 메뉴 — 을 그대로 잇고, 새로움은 오직 마감의 품질로만
드러낸다: 앱 강조색을 따라 물드는 텍스트 선택 영역, 하나의 진입 모션을 공유하는 모든 메뉴,
4.5:1을 지키는 모든 글자.

시각적 뿌리는 Microsoft Fluent의 밝은 사무 환경이다. 따뜻한 회색 중립 축(#faf9f8 →
#f3f2f1 → #e1dfdd) 위에 앱마다 하나의 강조색이 얹힌다 — Deck의 빨강, Doc의 파랑, Grid의
초록. 크롬(리본·패널·상태 표시줄)은 밀도가 높고 평평하며, 문서(캔버스·종이·시트)만이 흰
표면과 그림자를 가지고 떠 있다. 화면의 주인공은 언제나 문서다.

컴포넌트의 성격은 **정직하고 재빠른** 것이다. 네이티브 툴바처럼 즉각 반응하고, 장식이 없으며,
누르면 무조건 그 일이 일어난다. 100ms를 넘는 색 트랜지션도, 목적 없는 모션도 없다.

**Key Characteristics:**
- Office의 자리와 이름을 그대로 잇는 고밀도 크롬 (26px 컨트롤, 1px 헤어라인)
- 앱별 단일 강조색(빨강·파랑·초록)이 타이틀바부터 텍스트 선택까지 일관되게 물듦
- 평평한 크롬 / 떠 있는 문서 — 그림자는 오직 "문서 위에 떠 있다"는 뜻
- 글꼴 하나(Pretendard), 언어 하나(한국어 Office 용어)
- 모든 일시 표면(메뉴·대화상자·토스트)이 공유하는 단일 진입 모션

## Colors

Fluent 계열의 따뜻한 회색 중립 축 위에, 앱마다 Office 삼형제의 색 기억을 잇는 강조색
하나가 얹히는 팔레트다.

### Primary
- **Deck 빨강** (#c43e1c): AI Deck(PowerPoint 대응)의 강조색. 타이틀바, 리본 활성 탭,
  선택 썸네일 테두리, 텍스트 선택 영역까지 Deck 안의 모든 강조가 이 색이다.
- **Doc 파랑** (#185abd): AI Doc(Word 대응)의 강조색이자 앱 밖(런처 진입 전) 기본값.
- **Grid 초록** (#107c41): AI Grid(Excel 대응)의 강조색. 셀 선택 테두리, 채우기 핸들,
  시트 탭 활성 색.

세 색은 `--accent` 하나로 앱 루트(`.app`)에서 전환된다. 컴포넌트는 개별 색을 직접 집지 말고
`var(--accent)` 또는 `color-mix(in srgb, var(--accent) N%, ...)`로 유도한다.

### Neutral
- **잉크** (#201f1e): 본문 텍스트. 순검정이 아닌 따뜻한 흑.
- **잉크 2** (#484644): 보조 텍스트 — 리본 탭, 설명문.
- **잉크 3** (#797775): 3차 텍스트 — 라벨, 메타, 플레이스홀더. 흰 바탕 위 4.5:1의 하한선.
- **표면** (#ffffff): 문서(캔버스·종이·시트)와 카드, 메뉴.
- **표면 2** (#faf9f8): 런처 배경, 노트 영역.
- **표면 3** (#f3f2f1): 앱 바탕(스테이지), 시트 머리글, hover 상태.
- **선** (#e1dfdd) / **선 2** (#edebe9): 1px 헤어라인. 크롬의 깊이는 전부 이 두 선이 만든다.

### Functional
- **포커스 파랑** (#0f6cbd): 키보드 포커스 링과 입력 포커스 테두리 전용. 앱 강조색과
  분리되어 있어 Deck의 빨강 위에서도 포커스가 구분된다.
- **위험 빨강** (#a4262c): 삭제·오류 전용. Deck 빨강과 혼동되지 않도록 채도가 낮다.

### Named Rules
**한 화면 한 강조색 규칙.** 한 화면에는 그 앱의 강조색 하나만 산다. 두 번째 유채색이
필요해 보이면 그것은 상태(위험·포커스)이거나 설계 오류다.

**Office 팔레트 규칙.** 사용자가 고르는 색(도형 채우기·글자색·셀 음영)은 Office의 테마 색
60 + 표준 색 10 팔레트에서 나온다. 내보낸 `.pptx`에서 같은 색이 나와야 하기 때문이다.
이 팔레트를 장식적으로 재해석하지 않는다.

## Typography

**Display/Body Font:** Pretendard Variable (앱에 번들, SIL OFL 1.1; 폴백 시스템 한글 스택)
**Mono Font:** Cascadia Mono (SF Mono, Menlo, Consolas 폴백)

**Character:** 하나의 글꼴이 UI와 문서를 모두 담당한다 — 중립적이고 조용한 한글 그로테스크.
목소리의 차이는 글꼴이 아니라 크기·굵기·자간으로만 낸다. 수식·셀 주소·파일 경로처럼
"기계가 읽는 텍스트"만 모노스페이스를 쓴다.

### Hierarchy
- **Display** (750, 31px, -0.025em): 런처 히어로 제목 전용. 앱 안에서는 등장하지 않는다.
- **Headline** (600, 16–17px): 대화상자 제목, 런처 섹션·카드 이름.
- **Title** (600–700, 11–12px, +0.04em, 대문자): 패널 머리글, 노트 라벨. 크롬의 구획 표지.
- **Body** (400, 13–14px, 1.5): 크롬 본문. 문서 본문은 별도로 15px/1.72 (`.md--doc`).
- **Label** (400, 10.5–11px): 리본 버튼 라벨과 그룹 이름. Office의 밀도를 만드는 크기.
- **Mono** (400, 11–13px, tabular-nums): 수식 입력줄, 셀 주소, 저장 포맷 패널, 파일 메타.

### Named Rules
**글꼴 하나 규칙.** 새 글꼴을 추가하지 않는다. 가져온 문서가 어떤 글꼴을 요구해도
Pretendard로 치환되며(치환 사실은 보고), UI가 두 번째 목소리를 갖는 순간 이 규칙의
근거(어디서나 같은 모양)가 무너진다.

**문서-수출 짝 규칙.** 문서 타이포 수치는 Rust 상수와 한 쌍이다 (`.md--doc` 15px ↔
`doc::BODY_PX`, 시트 셀 ↔ `grid::CELL_PX`, 제목 배율 ↔ `mdblocks::HEADING_EM`).
한쪽만 바꾸면 내보낸 문서의 크기가 달라진다 — 스타일시트 단독 변경 금지.

## Layout

고정 크롬 + 유동 문서의 앱 셸이다. 위에서부터: 타이틀바(48px, 강조색), 리본(탭줄 34px +
본문 62px+, 흰 배경, 좁으면 가로 스크롤), 작업 영역(flex), 상태 표시줄(26px, 강조색).
작업 영역 안은 왼쪽 패널(208px; 슬라이드 정렬기·개요) · 스테이지(유동, #f3f2f1 바탕) ·
오른쪽 패널(280px; 저장 포맷 패널은 400px)의 3열이다.

밀도는 데스크톱 도구의 것이다: 컨트롤 높이 26px, 리본 버튼 최소 44px 폭, 패딩 리듬은
4 / 8 / 12 / 16px. 문서(캔버스·종이)는 스테이지 중앙에 28px 여백을 두고 떠 있으며, 줌은
transform scale로 처리한다. 런처만 예외적으로 콘텐츠 폭 1080px의 여유로운 페이지 레이아웃을
쓴다 (히어로 → 새로 만들기 카드 그리드(240px min, auto-fit) → 최근 문서 목록).

반응형은 "줄이면 스크롤"이다: 리본은 `overflow-x: auto`로 가로 스크롤되고, 메뉴·팔레트는
리본 안이 아니라 `document.body`에 그려져 잘리지 않는다(뷰포트 가장자리에서 뒤집힘).

## Elevation & Depth

**크롬은 평평하고, 그림자는 떠 있는 것에만.** 리본·패널·상태 표시줄은 그림자 없이 1px
헤어라인으로만 구획된다. 그림자를 가진 것은 문서 위에 떠 있는 것들뿐이다: 슬라이드 캔버스와
문서 종이(스테이지 위에), 메뉴·팔레트·대화상자·토스트(모든 것 위에), 시트 위의 차트.
대화상자 오버레이는 rgba(32,31,30,.4) + 2px 배경 블러로 방의 초점을 옮긴다.

### Shadow Vocabulary
- **shadow-1** (`0 1px 2px rgba(0,0,0,.08), 0 2px 6px rgba(0,0,0,.06)`): 낮게 뜬 것 —
  카드, 시트 위 차트, 수식 힌트.
- **shadow-2** (`0 4px 12px rgba(0,0,0,.12), 0 12px 28px rgba(0,0,0,.12)`): 높이 뜬 것 —
  캔버스·종이, 메뉴, 대화상자, 토스트.

### Named Rules
**뜨는 것만 그림자 규칙.** 그림자는 장식이 아니라 "문서 위에 떠 있다"는 뜻이다. 크롬에는
절대 그림자를 주지 않고, hover로 그림자를 켜지 않는다 (유일한 예외: 런처의 새 문서 카드
리프트 — 앱 진입 전의 페이지이므로).

## Shapes

작고 정직한 반경의 형태 언어다. 컨트롤은 4px(`--radius`), 떠 있는 메뉴·팝오버는
6px(`--radius-md`), 카드·대화상자·파일 목록은 8px(`--radius-lg`), 배지·칩·스크롤바 썸은
pill(999px). 리본 탭과 시트 탭은 위쪽만 둥근 사다리꼴(4px 4px 0 0)로 Office의 탭 문법을
따른다. 테두리는 어디서나 1px 헤어라인이고, 두꺼운 유채색 테두리는 선택 상태(2px)에만
허용된다. 마크다운 인용 문단의 왼쪽 3px 회색 선은 문서 조판 관례로서의 예외다.

## Components

컴포넌트의 성격: **정직하고 재빠른** — 네이티브 툴바처럼 즉각 반응하고, 장식이 없고,
누르면 그 일이 일어난다. 모든 상호작용 상태 변화는 100ms 색 트랜지션 하나로 통일된다.

### Buttons
- **Shape:** 작게 둥근 모서리 (4px)
- **Primary** (`.btn--primary`): 앱 강조색 배경 + 흰 글자, 7px 16px 패딩. 대화상자의 확인
  동작 전용. hover는 밝기 8% 상승.
- **Default** (`.btn`): 흰 배경 + 1px 선 테두리. hover에 #f3f2f1.
- **Danger** (`.btn--danger`): #a4262c 배경. 삭제 확인 전용.
- **리본 버튼** (`.rbtn`): 투명 배경에서 시작해 hover에 배경+테두리가 나타나는 아이콘 위
  라벨 스택(최소 44px 폭, 11px 라벨). 눌린 상태(`aria-pressed`)는 #edebe9. Office의 큰
  버튼/작은 버튼(`.rbtn--sm`, 가로 배열 26px) 두 체급.
- **Focus:** 모든 버튼 공통 2px #0f6cbd 링. 리본처럼 잘리는 컨테이너 안에서는 내향(-2px).

### Chips
- **Style:** #f3f2f1 배경, #484644 글자, 1px 선 테두리, pill 반경, 3px 8px 패딩.
- **State:** 선택 시(`aria-pressed`) 앱 강조색 배경 + 흰 글자.

### Cards / Containers
- **Corner Style:** 8px
- **Background:** 흰 표면 + 1px 선 테두리
- **Shadow Strategy:** shadow-1이 기본, 런처 새 문서 카드만 hover에 -2px 리프트 + shadow-2
  + 강조색 32% 테두리
- **Internal Padding:** 18px (런처 카드), 12px (패널 섹션)

### Inputs / Fields
- **Style:** 흰 배경, 1px 선 테두리, 4px 반경, 26px 높이(크롬 표준) / 32px(대화상자)
- **Focus:** 테두리가 #0f6cbd로 바뀌고 같은 색 1px 링(box-shadow)이 겹쳐 2px 두께가 된다
- **기계 텍스트:** 수식 입력줄·이름 상자·셀 편집기는 모노스페이스 + tabular-nums
- **Disabled:** opacity 0.4–0.5, 커서 default

### Navigation (리본)
- 탭줄 34px: 13px 텍스트, 활성 탭은 강조색 글자 + 굵기 600 + 아래 2px 강조색 언더라인.
  상황별 탭(도형 서식·표 디자인)은 오른쪽에 1px 구분선을 두고 강조색 라벨 아래 붙는다.
- 그룹: 1px 헤어라인으로 구획, 아래에 10.5px 회색 그룹 이름 — Office의 문법 그대로.

### Signature Component: { } 저장 포맷 패널
오른쪽 400px 패널. 지금 이 순간 디스크에 쓰일 md/json을 실제 저장 경로와 같은 Rust
직렬화기로 보여준다. VS Code 밝은 테마 계열의 구문 토큰 색(#0451a5 키, #a31515 문자열,
#098658 숫자, #6a9955 주석)에 11.5px 모노스페이스. 이 제품의 주장("화면과 디스크가 어긋날
수 없다")이 시각화되는 곳이므로, 장식 없이 코드 그 자체로 보여야 한다.

### 일시 표면 공통 (메뉴 · 팝오버 · 대화상자 · 토스트)
`document.body`에 그려지고, 6–8px 반경 + shadow-2, 그리고 **하나의 진입 모션**을 공유한다:
4–8px 상승 + 페이드, 140–180ms, `cubic-bezier(.16,1,.3,1)`. `prefers-reduced-motion`이
켜져 있으면 전부 즉시 나타난다.

## Do's and Don'ts

### Do:
- **Do** 새 컨트롤은 Office에 있는 자리, Office의 한국어 이름으로 넣는다 — 리본 탭·그룹의
  이름과 순서는 스모크 테스트가 검증하는 계약이다.
- **Do** 강조가 필요한 곳은 `var(--accent)` 또는 `color-mix(in srgb, var(--accent) N%, ...)`로
  유도한다. 앱이 바뀌면 색이 따라 바뀌어야 한다.
- **Do** 텍스트 선택·캐럿·체크박스·스크롤바 같은 브라우저 표면도 팔레트로 테마한다.
- **Do** 새 일시 표면(메뉴류)은 `Popover`를 재사용하고 공유 진입 모션을 상속한다.
- **Do** 본문·플레이스홀더 4.5:1, 큰 글자 3:1의 대비 하한을 지킨다 (#797775가 흰 바탕의 하한).

### Don't:
- **Don't** 크롬(리본·패널·상태 표시줄)에 그림자를 주지 않는다. 그림자는 "떠 있다"는 뜻이다.
- **Don't** 두 번째 글꼴, 그라데이션 장식, 유리 효과를 들이지 않는다. 유일한 블러는 대화상자
  오버레이의 2px이다.
- **Don't** 고정 유채색 틴트(#d6e4f7 같은)를 쓰지 않는다 — 앱 강조색과 싸운다. 틴트는
  color-mix로 유도한다.
- **Don't** `.md--doc`의 문서 타이포 수치를 Rust 상수(`BODY_PX` 등) 없이 단독 변경하지
  않는다. 내보낸 문서의 크기가 달라진다.
- **Don't** 사용자 색 팔레트(테마 60 + 표준 10)를 재구성하지 않는다. 내보낸 Office 파일의
  색 일치가 존재 이유다.
