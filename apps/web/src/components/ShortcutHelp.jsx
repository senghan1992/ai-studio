import React, { useEffect } from 'react';

/**
 * The keyboard shortcut sheet, opened with F1 or Ctrl+/.
 *
 * Office has had F1 since Office 4, and an Office user's first move in an
 * unfamiliar suite is to press it and find out which of their habits still
 * work. Listing them app by app is also the honest answer to "what does this
 * program not have" — anything missing from here is missing.
 */

const COMMON = [
  ['Ctrl+S', '저장 (2.5초 후 자동 저장도 함께 동작)'],
  ['Ctrl+Z · Ctrl+Y', '실행 취소 · 다시 실행'],
  ['Ctrl+F · Ctrl+H', '찾기 · 바꾸기'],
  ['F1', '이 목록'],
];

const BY_APP = {
  deck: {
    label: 'AI Deck · PowerPoint',
    groups: [
      {
        label: '슬라이드',
        rows: [
          ['Ctrl+M', '새 슬라이드'],
          ['PageUp · PageDown', '이전 · 다음 슬라이드'],
          ['F5', '처음부터 슬라이드 쇼'],
          ['Shift+F5', '현재 슬라이드부터'],
          ['Ctrl+P', '인쇄 · PDF (슬라이드마다 한 장)'],
          ['Delete', '축소판에서 슬라이드 삭제'],
        ],
      },
      {
        label: '개체',
        rows: [
          ['Tab · Shift+Tab', '슬라이드의 개체를 차례로 선택'],
          ['방향키', '8px씩 이동'],
          ['Ctrl+방향키', '1px씩 미세 이동'],
          ['Alt+방향키', '크기 조절'],
          ['Enter', '선택한 개체의 텍스트 편집'],
          ['Ctrl+D', '복제'],
          ['Ctrl+C · X · V', '복사 · 잘라내기 · 붙여넣기'],
          ['Esc', '선택 해제 · 편집 종료'],
          ['Shift+드래그(회전)', '15°씩 회전'],
          ['Alt+드래그', '안내선 스냅 없이 이동'],
        ],
      },
      {
        label: '글자',
        rows: [
          ['Ctrl+B · Ctrl+I', '굵게 · 기울임 (마크다운 표기로 들어갑니다)'],
          ['Ctrl+L · E · R · J', '왼쪽 · 가운데 · 오른쪽 · 양쪽 맞춤'],
        ],
      },
      {
        label: '슬라이드 쇼 중',
        rows: [
          ['→ · Space · PageDown', '다음'],
          ['← · Backspace', '이전'],
          ['Home · End', '첫 · 마지막 슬라이드'],
          ['B · W', '검은 화면 · 흰 화면 (다시 누르면 복귀)'],
          ['S', '발표자 노트'],
          ['Esc', '끝내기'],
        ],
      },
    ],
  },
  doc: {
    label: 'AI Doc · Word',
    groups: [
      {
        label: '글자',
        rows: [
          ['Ctrl+B · Ctrl+I', '굵게 · 기울임'],
          ['Ctrl+U', '밑줄'],
          ['Ctrl+Shift+> · <', '글자 크게 · 작게'],
          ['Ctrl+K', '하이퍼링크'],
        ],
      },
      {
        label: '문단',
        rows: [
          ['Ctrl+L · E · R · J', '왼쪽 · 가운데 · 오른쪽 · 양쪽 맞춤'],
          ['Ctrl+1 · 5 · 2', '줄 간격 1.0 · 1.5 · 2.0'],
          ['Tab · Shift+Tab', '들여쓰기 · 내어쓰기 (목록에서는 수준 변경)'],
          ['Ctrl+Alt+1 · 2 · 3', '제목 1 · 2 · 3'],
          ['Ctrl+Shift+N', '본문 스타일'],
          ['Enter', '문단 나누기'],
          ['Backspace(문단 첫머리)', '앞 문단과 합치기'],
        ],
      },
      {
        label: '이동',
        rows: [
          ['↑ · ↓', '문단 안에서, 그리고 문단을 넘어 이동'],
          ['Ctrl+Home · Ctrl+End', '문서 처음 · 끝'],
          ['Ctrl+Enter', '페이지 나누기'],
          ['Ctrl+P', '인쇄 · PDF'],
        ],
      },
    ],
  },
  grid: {
    label: 'AI Grid · Excel',
    groups: [
      {
        label: '이동과 선택',
        rows: [
          ['방향키', '한 칸 이동'],
          ['Ctrl+방향키', '데이터 끝으로 건너뛰기'],
          ['Shift+방향키', '선택 확장'],
          ['Ctrl+Shift+방향키', '데이터 끝까지 선택'],
          ['Ctrl+Home · Ctrl+End', 'A1 · 마지막 데이터 셀'],
          ['Home · End', '행의 처음 · 마지막 데이터 열'],
          ['PageUp · PageDown', '한 화면씩'],
          ['Ctrl+PageUp · PageDown', '이전 · 다음 시트'],
          ['Ctrl+P', '인쇄 · PDF (사용 중인 범위)'],
          ['Ctrl+A', '현재 데이터 블록, 다시 누르면 시트 전체'],
          ['Ctrl+Space · Shift+Space', '열 전체 · 행 전체 선택'],
        ],
      },
      {
        label: '입력과 편집',
        rows: [
          ['F2 · Enter', '셀 편집'],
          ['Alt+Enter', '셀 안에서 줄 바꾸기'],
          ['F4', '수식의 참조를 $A$1 ↔ A1 로 전환'],
          ['Ctrl+D · Ctrl+R', '아래로 · 오른쪽으로 채우기'],
          ['Ctrl+; · Ctrl+Shift+;', '오늘 날짜 · 지금 시각'],
          ['Delete', '내용 지우기 (서식은 남김)'],
          ['Ctrl+C · Ctrl+Shift+C', '값 복사 · 수식 복사'],
          ['Ctrl+V', '붙여넣기 (Excel에서 그대로 붙습니다)'],
        ],
      },
      {
        label: '서식',
        rows: [
          ['Ctrl+B · I · U', '굵게 · 기울임 · 밑줄'],
          ['Ctrl+1', '셀 서식'],
          ['Ctrl+Shift+1 · 4 · 5', '쉼표 · 통화 · 백분율'],
          ['Ctrl+`', '수식 보기 / 값 보기'],
        ],
      },
      {
        label: '마우스',
        rows: [
          ['머리글 경계 두 번 누르기', '너비 · 높이 자동 맞춤'],
          ['머리글 끌기', '여러 행 · 열 선택'],
          ['선택 영역 우하단 손잡이 끌기', '자동 채우기'],
          ['시트 탭 끌기', '시트 순서 변경'],
        ],
      },
    ],
  },
};

export default function ShortcutHelp({ type, onClose }) {
  useEffect(() => {
    const onKey = (e) => {
      if (e.key === 'Escape' || e.key === 'F1') {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  const app = BY_APP[type] ?? BY_APP.doc;

  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dialog dialog--wide" role="dialog" aria-modal="true" aria-label="키보드 단축키">
        <div className="dialog__head">키보드 단축키 — {app.label}</div>
        <div className="dialog__body shortcuts">
          <section className="shortcuts__group">
            <h3>공통</h3>
            <Rows rows={COMMON} />
          </section>
          {app.groups.map((group) => (
            <section key={group.label} className="shortcuts__group">
              <h3>{group.label}</h3>
              <Rows rows={group.rows} />
            </section>
          ))}
        </div>
        <div className="dialog__foot">
          <button type="button" className="btn btn--primary" onClick={onClose}>
            닫기
          </button>
        </div>
      </div>
    </div>
  );
}

function Rows({ rows }) {
  return (
    <dl className="shortcuts__list">
      {rows.map(([keys, what]) => (
        <React.Fragment key={keys}>
          <dt>
            {keys.split(' · ').map((k, i) => (
              <React.Fragment key={k}>
                {i > 0 && <span className="shortcuts__or"> · </span>}
                <kbd>{k}</kbd>
              </React.Fragment>
            ))}
          </dt>
          <dd>{what}</dd>
        </React.Fragment>
      ))}
    </dl>
  );
}
