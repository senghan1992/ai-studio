import React, { useEffect, useRef, useState } from 'react';
import { api } from '../api.js';
import { Dialog } from './ui.jsx';
import {
  OFFICE_ACCEPT, hasNativePicker, importFile, isOfficeFile, pickAndImport, rejectionFor,
} from '../lib/importFile.js';

const APPS = [
  {
    type: 'deck',
    name: 'AI Deck',
    short: 'D',
    color: 'var(--deck)',
    desc: '슬라이드마다 내용 마크다운과 좌표 JSON을 한 쌍으로 저장하는 프레젠테이션.',
    files: 'slides/01-표지.md + .layout.json',
  },
  {
    type: 'doc',
    name: 'AI Doc',
    short: 'W',
    color: 'var(--doc)',
    desc: '순수 마크다운으로 흐르는 문서. 서식을 준 문단만 JSON에 기록됩니다.',
    files: 'content/01-서론.md + .meta.json',
  },
  {
    type: 'grid',
    name: 'AI Grid',
    short: 'X',
    color: 'var(--grid)',
    desc: 'JSON이 수식의 원천, 마크다운은 AI가 읽는 표 투영. 둘 다 저장됩니다.',
    files: 'sheets/01-시트1.md + .cells.json',
  },
];

const APP_BY_TYPE = Object.fromEntries(APPS.map((a) => [a.type, a]));

export default function Launcher({ onOpen, onError, notify }) {
  const [projects, setProjects] = useState([]);
  const [workspace, setWorkspace] = useState('');
  const [loading, setLoading] = useState(true);
  const [creating, setCreating] = useState(null);
  const [title, setTitle] = useState('');
  const [busy, setBusy] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(null);
  const [importing, setImporting] = useState(null);
  const [dragging, setDragging] = useState(false);
  /** What an import had to convert, shown before the project opens. */
  const [report, setReport] = useState(null);
  const fileInput = useRef(null);

  const refresh = () => {
    setLoading(true);
    api
      .listProjects()
      .then((data) => {
        setProjects(data.projects ?? []);
        setWorkspace(data.workspace ?? '');
      })
      .catch((e) => onError(e.message))
      .finally(() => setLoading(false));
  };

  useEffect(refresh, []);

  const create = async () => {
    if (!creating) return;
    setBusy(true);
    try {
      const project = await api.createProject(creating.type, title.trim() || undefined);
      setCreating(null);
      setTitle('');
      onOpen(project.folder, project.type);
    } catch (e) {
      onError(e.message);
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    if (!confirmDelete) return;
    setBusy(true);
    try {
      await api.deleteProject(confirmDelete.folder);
      notify(`${confirmDelete.title} 삭제됨`);
      setConfirmDelete(null);
      refresh();
    } catch (e) {
      onError(e.message);
    } finally {
      setBusy(false);
    }
  };

  /**
   * Open an existing Office file.
   *
   * Reported rather than silent when something could not be carried across: a
   * user who is told "그라데이션 채우기는 첫 색으로 단순화했습니다" knows what to
   * check, where silence leaves them comparing slides by eye.
   */
  const runImport = async (run, label) => {
    setImporting(label);
    try {
      const result = await run();
      if (!result) return;
      refresh();
      const notes = result.warnings ?? [];
      if (notes.length === 0) {
        notify(`${result.folder}로 가져왔습니다`);
        onOpen(result.folder, result.project.type);
        return;
      }
      // A toast would be gone before six lines could be read, and these are the
      // lines that say which parts of the file look different — the same job
      // Word's own converter dialog does.
      setReport({ notes, folder: result.folder, type: result.project.type });
    } catch (e) {
      onError(e.message);
    } finally {
      setImporting(null);
    }
  };

  const openOffice = () => {
    if (hasNativePicker()) {
      runImport(() => pickAndImport(), 'Office 문서');
      return;
    }
    fileInput.current?.click();
  };

  const onFiles = (files) => {
    const file = [...(files ?? [])][0];
    if (!file) return;
    if (!isOfficeFile(file.name)) {
      onError(rejectionFor(file.name));
      return;
    }
    runImport(() => importFile(file), file.name);
  };

  return (
    <div
      className={`launcher${dragging ? ' is-dropping' : ''}`}
      onDragOver={(e) => {
        if (![...e.dataTransfer.types].includes('Files')) return;
        e.preventDefault();
        setDragging(true);
      }}
      onDragLeave={(e) => {
        // Leaving for a child element is not leaving the drop zone.
        if (e.currentTarget.contains(e.relatedTarget)) return;
        setDragging(false);
      }}
      onDrop={(e) => {
        e.preventDefault();
        setDragging(false);
        onFiles(e.dataTransfer.files);
      }}
    >
      <input
        ref={fileInput}
        type="file"
        accept={OFFICE_ACCEPT}
        hidden
        onChange={(e) => {
          onFiles(e.target.files);
          e.target.value = '';
        }}
      />
      <div className="launcher__inner">
        <header className="launcher__hero">
          <h1>AI Studio</h1>
          <p>
            Office와 같은 방식으로 만들고, AI가 읽을 수 있는 방식으로 저장합니다. 모든 문서는 zip이 아닌
            일반 폴더이며, 내용은 마크다운, 배치와 계산은 JSON으로 나뉘어 들어갑니다. 저장할 때마다 전체
            내용을 요약한 <code>AI.md</code>가 함께 생성되어 RAG에 그대로 넣을 수 있습니다.
          </p>
        </header>

        <div className="launcher__grid">
          <button className="newcard newcard--open" onClick={openOffice} disabled={!!importing}>
            <span className="newcard__icon newcard__icon--open">↥</span>
            <span className="newcard__name">
              {importing ? `${importing} 가져오는 중…` : '기존 파일 열기'}
            </span>
            <span className="newcard__desc">
              PowerPoint · Word · Excel 파일을 이 포맷으로 가져옵니다. 원본은 그대로 두고 폴더 문서를
              새로 만듭니다.
            </span>
            <span className="newcard__files">.pptx · .docx · .xlsx {hasNativePicker() ? '' : '· 끌어다 놓기'}</span>
          </button>
          {APPS.map((app) => (
            <button
              key={app.type}
              className="newcard"
              onClick={() => {
                setCreating(app);
                setTitle('');
              }}
            >
              <span className="newcard__icon" style={{ background: app.color }}>
                {app.short}
              </span>
              <span className="newcard__name">{app.name}</span>
              <span className="newcard__desc">{app.desc}</span>
              <span className="newcard__files">{app.files}</span>
            </button>
          ))}
        </div>

        <div className="section-head">
          <h2>최근 문서</h2>
          {workspace && <span>{workspace}</span>}
        </div>

        {loading ? (
          <div className="empty">
            <div className="spinner" style={{ margin: '0 auto 10px' }} />
            불러오는 중…
          </div>
        ) : projects.length === 0 ? (
          <div className="filelist">
            <div className="empty">아직 문서가 없습니다. 위에서 하나 만들어 보세요.</div>
          </div>
        ) : (
          <div className="filelist">
            {projects.map((p) => {
              const app = APP_BY_TYPE[p.type];
              return (
                <div key={p.folder} style={{ display: 'flex', alignItems: 'stretch' }}>
                  <button className="filerow" onClick={() => onOpen(p.folder, p.type)}>
                    <span className="filerow__icon" style={{ background: app?.color }}>
                      {app?.short}
                    </span>
                    <span className="filerow__main">
                      <span className="filerow__name">{p.title}</span>
                      <br />
                      <span className="filerow__meta">
                        {p.folder} · {p.label} {p.count}개 · {formatDate(p.modified)}
                      </span>
                    </span>
                  </button>
                  <button
                    className="filerow__del"
                    title="삭제"
                    aria-label={`${p.title} 삭제`}
                    onClick={() => setConfirmDelete(p)}
                  >
                    ×
                  </button>
                </div>
              );
            })}
          </div>
        )}
      </div>

      {creating && (
        <Dialog
          title={`새 ${creating.name}`}
          confirmLabel="만들기"
          busy={busy}
          onCancel={() => setCreating(null)}
          onConfirm={create}
        >
          <p>
            <code>{creating.files.split(' + ')[0].split('/')[0]}/</code> 폴더에 마크다운과 JSON 한 쌍으로
            저장됩니다.
          </p>
          <input
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder="문서 제목"
            aria-label="문서 제목"
          />
        </Dialog>
      )}

      {report && (
        <Dialog
          title="가져왔습니다 — 바뀐 것"
          confirmLabel="열기"
          onCancel={() => setReport(null)}
          onConfirm={() => {
            const open = report;
            setReport(null);
            onOpen(open.folder, open.type);
          }}
        >
          <p style={{ margin: '0 0 8px', fontSize: 13, color: 'var(--ink-2)' }}>
            <code>{report.folder}</code>로 가져왔습니다. 아래는 이 프로그램에 맞게{' '}
            <strong>바꾼 것</strong>과 <strong>옮기지 못한 것</strong>입니다. 나머지는 원본
            그대로입니다.
          </p>
          <ul className="importnotes">
            {report.notes.map((note) => (
              <li key={note}>{note}</li>
            ))}
          </ul>
        </Dialog>
      )}

      {confirmDelete && (
        <Dialog
          title="문서를 삭제할까요?"
          confirmLabel="삭제"
          danger
          busy={busy}
          onCancel={() => setConfirmDelete(null)}
          onConfirm={remove}
        >
          <p>
            <strong>{confirmDelete.title}</strong> 폴더(<code>{confirmDelete.folder}</code>)와 그 안의 모든
            파일이 영구적으로 삭제됩니다. 되돌릴 수 없습니다.
          </p>
        </Dialog>
      )}
    </div>
  );
}

function formatDate(iso) {
  if (!iso) return '수정 시각 없음';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '수정 시각 없음';
  return d.toLocaleString('ko-KR', { dateStyle: 'medium', timeStyle: 'short' });
}

