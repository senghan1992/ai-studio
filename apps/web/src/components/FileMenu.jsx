import React, { useState } from 'react';
import { api } from '../api.js';
import { Group, Btn, Dialog, Field } from './ui.jsx';
import { EXPORT_TARGETS, downloadExport } from '../lib/exports.js';

/**
 * The ribbon's File tab.
 *
 * Office puts new / open / save / save-as / export / print / close here, and
 * people look for them here. Rendering it as a ribbon tab rather than a takeover
 * screen keeps one interaction model for the whole window.
 */
export default function FileMenu({
  project, dirty, onSave, onHome, onNewProject, onPrint, notify,
}) {
  const [copyDialog, setCopyDialog] = useState(null);
  const [busy, setBusy] = useState(false);
  const [exporting, setExporting] = useState(null);
  /** null = closed, 'loading', or { snapshots, keep } from the server. */
  const [history, setHistory] = useState(null);
  const [restoring, setRestoring] = useState(null);

  const type = project.type;
  const folder = project.folder;
  const targets = EXPORT_TARGETS[type] ?? [];

  const openHistory = async () => {
    setHistory('loading');
    try {
      setHistory(await api.history(folder));
    } catch (e) {
      setHistory(null);
      notify(e.message);
    }
  };

  const restore = async (snapshot) => {
    setRestoring(snapshot.name);
    try {
      // Flush the current state first so the trail keeps what the screen shows.
      if (dirty) await onSave?.();
      await api.restore(folder, snapshot.name);
      // Reload rather than patch: a restore replaces the whole document, and
      // the undo stack, the save-format panel and the cursor all belong to the
      // old one. Word closes and reopens too.
      window.location.reload();
    } catch (e) {
      setRestoring(null);
      notify(e.message);
    }
  };

  const saveCopy = async () => {
    const title = copyDialog.title.trim();
    if (!title) return;
    setBusy(true);
    try {
      const created = await api.createProject(type, title, false);
      const key = { deck: 'slides', doc: 'sections', grid: 'sheets' }[type];
      await api.saveProject(created.folder, { manifest: { title }, [key]: project[key] });
      setCopyDialog(null);
      notify(`사본을 만들었습니다: ${created.folder}`);
    } catch (e) {
      notify(e.message);
    } finally {
      setBusy(false);
    }
  };

  const runExport = async (target) => {
    setExporting(target.ext);
    try {
      await downloadExport(folder, target.ext, project.manifest?.title);
      notify(`${target.label} 파일을 내려받았습니다`);
    } catch (e) {
      notify(e.message);
    } finally {
      setExporting(null);
    }
  };

  return (
    <>
      <Group label="문서">
        <Btn icon="🞤" label="새로 만들기" onClick={() => onNewProject?.()} />
        <Btn icon="📂" label="열기" title="시작 화면으로 돌아갑니다" onClick={onHome} />
        <Btn icon="💾" label="저장" onClick={onSave} disabled={!dirty} title="Ctrl+S" />
        <Btn
          icon="⧉"
          label="사본 만들기"
          onClick={() => setCopyDialog({ title: `${project.manifest?.title ?? '문서'} 사본` })}
        />
        <Btn
          icon="🕘"
          label="버전 기록"
          title="이전에 저장된 버전을 보고 되돌립니다"
          onClick={openHistory}
        />
      </Group>

      <Group label="내보내기">
        {targets.map((target) => (
          <Btn
            key={target.ext}
            icon={target.icon}
            label={exporting === target.ext ? '내보내는 중…' : target.label}
            title={`${target.description} — 서버에서 변환해 내려받습니다`}
            disabled={!!exporting}
            onClick={() => runExport(target)}
          />
        ))}
      </Group>

      {onPrint && (
        <Group label="인쇄">
          <Btn icon="🖨" label="인쇄" title="Ctrl+P" onClick={onPrint} />
        </Group>
      )}

      <Group label="정보">
        <div style={{ fontSize: 11.5, color: 'var(--ink-2)', lineHeight: 1.7, padding: '2px 6px', maxWidth: 320 }}>
          <div>
            폴더 <code>{folder}</code>
          </div>
          <div>
            만든 날짜 {formatDate(project.manifest?.created)}
          </div>
          <div>
            형식 <code>{project.manifest?.format}</code> v{project.manifest?.formatVersion}
          </div>
        </div>
      </Group>

      {history !== null && (
        <Dialog
          title="버전 기록"
          confirmLabel="닫기"
          onCancel={() => setHistory(null)}
          onConfirm={() => setHistory(null)}
        >
          {history === 'loading' ? (
            <p>불러오는 중…</p>
          ) : history.snapshots?.length ? (
            <>
              <p>
                저장할 때마다 직전 상태가 보관됩니다 (최근 {history.keep}개). 복원해도 지금
                상태가 먼저 보관되므로, 복원 자체도 이 목록에서 되돌릴 수 있습니다.
              </p>
              <ul className="versions">
                {history.snapshots.map((snapshot) => (
                  <li key={snapshot.name} className="versions__row">
                    <span className="versions__main">
                      <span className="versions__when">{formatSavedAt(snapshot.savedAt)}</span>
                      <span className="versions__meta">
                        {snapshot.title} · 파일 {snapshot.files}개 · {formatBytes(snapshot.bytes)}
                      </span>
                    </span>
                    <button
                      type="button"
                      className="btn"
                      disabled={!!restoring}
                      onClick={() => restore(snapshot)}
                    >
                      {restoring === snapshot.name ? '복원 중…' : '복원'}
                    </button>
                  </li>
                ))}
              </ul>
            </>
          ) : (
            <p>아직 보관된 버전이 없습니다. 문서를 고쳐 저장하면 직전 상태가 여기에 남습니다.</p>
          )}
        </Dialog>
      )}

      {copyDialog && (
        <Dialog
          title="사본 만들기"
          confirmLabel="만들기"
          busy={busy}
          onCancel={() => setCopyDialog(null)}
          onConfirm={saveCopy}
        >
          <p>현재 내용을 새 프로젝트 폴더로 복사합니다. 원본은 그대로 남습니다.</p>
          <Field label="새 문서 제목">
            <input value={copyDialog.title} onChange={(e) => setCopyDialog({ title: e.target.value })} />
          </Field>
        </Dialog>
      )}
    </>
  );
}

function formatDate(iso) {
  if (!iso) return '—';
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? '—' : d.toLocaleDateString('ko-KR', { dateStyle: 'medium' });
}

function formatSavedAt(iso) {
  const d = new Date(iso);
  return Number.isNaN(d.getTime())
    ? iso
    : d.toLocaleString('ko-KR', { dateStyle: 'medium', timeStyle: 'medium' });
}

function formatBytes(n) {
  if (!Number.isFinite(n)) return '';
  if (n < 1024) return `${n}B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)}KB`;
  return `${(n / (1024 * 1024)).toFixed(1)}MB`;
}
