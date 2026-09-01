import React, { useEffect, useRef } from 'react';

/**
 * Find / Replace bar, shared by the Doc and Grid editors.
 *
 * Modeled on Office's floating dialog rather than a browser find bar: Enter goes
 * to the next hit, Shift+Enter to the previous, and the count reads "3 / 12" so
 * you know where you are without counting highlights.
 */
export default function FindBar({
  state, hits, onChange, onNext, onPrev, onReplace, onReplaceAll, onClose,
}) {
  const inputRef = useRef(null);

  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);

  const total = hits?.length ?? 0;
  const position = state.at === null || state.at === undefined ? null : state.at + 1;

  const onKeyDown = (e) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      if (e.shiftKey) onPrev();
      else onNext();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      onClose();
    }
  };

  return (
    <div className="findbar" role="search" aria-label="찾기 및 바꾸기">
      <input
        ref={inputRef}
        type="text"
        value={state.query}
        placeholder="찾을 내용"
        aria-label="찾을 내용"
        onChange={(e) => onChange({ query: e.target.value })}
        onKeyDown={onKeyDown}
      />

      <span className="findbar__count">
        {state.query === '' ? '' : total === 0 ? '결과 없음' : `${position ?? '–'} / ${total}`}
      </span>

      <button type="button" onClick={onPrev} disabled={!total} title="이전 (Shift+Enter)">
        ↑
      </button>
      <button type="button" onClick={onNext} disabled={!total} title="다음 (Enter)">
        ↓
      </button>

      {state.replace ? (
        <>
          <input
            type="text"
            value={state.replacement ?? ''}
            placeholder="바꿀 내용"
            aria-label="바꿀 내용"
            onChange={(e) => onChange({ replacement: e.target.value })}
            onKeyDown={onKeyDown}
          />
          <button type="button" onClick={onReplace} disabled={!total} title="현재 항목만 바꾸기">
            바꾸기
          </button>
          <button type="button" onClick={onReplaceAll} disabled={!total} title="모두 바꾸기">
            모두 바꾸기
          </button>
        </>
      ) : (
        <button type="button" onClick={() => onChange({ replace: true })} title="Ctrl+H">
          바꾸기…
        </button>
      )}

      <label title="대소문자를 구분합니다">
        <input
          type="checkbox"
          checked={!!state.matchCase}
          onChange={(e) => onChange({ matchCase: e.target.checked })}
        />
        대/소문자
      </label>

      {state.inFormulas !== undefined && (
        <label title="표시된 값 대신 수식 문자열을 검색합니다">
          <input
            type="checkbox"
            checked={!!state.inFormulas}
            onChange={(e) => onChange({ inFormulas: e.target.checked })}
          />
          수식 검색
        </label>
      )}

      <button type="button" className="findbar__close" onClick={onClose} title="닫기 (Esc)" aria-label="닫기">
        ×
      </button>
    </div>
  );
}
