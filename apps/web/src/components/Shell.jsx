import React, { useEffect, useState } from 'react';

import ShortcutHelp from './ShortcutHelp.jsx';

const APP_META = {
  deck: { name: 'AI Deck', short: 'D', color: 'var(--deck)' },
  doc: { name: 'AI Doc', short: 'W', color: 'var(--doc)' },
  grid: { name: 'AI Grid', short: 'X', color: 'var(--grid)' },
};

/**
 * Chrome shared by all three editors: title bar, ribbon slot, work area, status
 * bar. Keeping it here means the three apps look and behave like one suite
 * without each re-implementing the shell.
 */
export default function Shell({
  type, title, onTitleChange, onTitleCommit,
  dirty, saving, savedAt, onSave, onHome,
  ribbon, status, children,
  inspectorOpen, onToggleInspector,
}) {
  const app = APP_META[type] ?? APP_META.doc;
  const [draft, setDraft] = useState(title ?? '');
  const [help, setHelp] = useState(false);

  useEffect(() => setDraft(title ?? ''), [title]);

  // F1 is the first key an Office user presses in a program they do not know.
  useEffect(() => {
    const onKey = (e) => {
      if (e.key === 'F1' || ((e.ctrlKey || e.metaKey) && e.key === '/')) {
        e.preventDefault();
        setHelp(true);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  return (
    <div className="app" style={{ '--accent': app.color }}>
      <header className="titlebar">
        <button className="titlebar__brand" onClick={onHome} title="시작 화면으로">
          <span className="titlebar__logo">{app.short}</span>
          <span>{app.name}</span>
        </button>

        <div className="titlebar__title">
          <input
            className="titlebar__name"
            value={draft}
            onChange={(e) => {
              setDraft(e.target.value);
              onTitleChange?.(e.target.value);
            }}
            onBlur={() => onTitleCommit?.(draft)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') e.currentTarget.blur();
            }}
            aria-label="문서 제목"
          />
          {/* The badge says what autosave is about to do, not only what it did:
              "저장되지 않음" on a document that saves itself in two seconds
              reads like a warning it is not. */}
          <span
            className="titlebar__badge"
            title={
              saving
                ? '디스크에 쓰는 중입니다'
                : dirty
                ? '편집을 멈추면 몇 초 안에 자동 저장됩니다. Ctrl+S로 바로 저장할 수 있습니다'
                : '모든 변경이 폴더에 기록되었습니다'
            }
          >
            {saving ? '저장 중…' : dirty ? '자동 저장 대기' : '저장됨'}
          </span>
        </div>

        <div className="titlebar__actions">
          <button className="tbtn tbtn--ghost" onClick={() => setHelp(true)} title="키보드 단축키 (F1)">
            ?
          </button>
          <button
            className={`tbtn tbtn--ghost${inspectorOpen ? ' tbtn--on' : ''}`}
            onClick={onToggleInspector}
            title="AI 저장 포맷 패널 표시/숨기기"
          >
            {'{ }'} 저장 포맷
          </button>
          <button className="tbtn" onClick={onSave} disabled={saving || !dirty}>
            저장
          </button>
        </div>
      </header>

      {ribbon}

      <div className="workarea">{children}</div>

      {help && <ShortcutHelp type={type} onClose={() => setHelp(false)} />}

      <footer className="statusbar">
        {status}
        <span className="statusbar__spacer" />
        <span>{savedAt ? `마지막 저장 ${formatTime(savedAt)}` : '아직 저장하지 않음'}</span>
      </footer>
    </div>
  );
}

function formatTime(iso) {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '—';
  return d.toLocaleTimeString('ko-KR', { hour: '2-digit', minute: '2-digit', second: '2-digit' });
}
