import { NavLink, Route, Routes, Link, useNavigate, useParams } from 'react-router-dom'
import { useCallback, useEffect, useState } from 'react'
import { api } from './api'
import './App.css'

function Shell({ children }) {
  return (
    <div className="shell">
      <header className="topbar">
        <Link to="/" className="brand">Flare</Link>
        <nav className="nav">
          <NavLink to="/" end className={({ isActive }) => (isActive ? 'active' : undefined)}>
            Projects
          </NavLink>
          <NavLink to="/new" className={({ isActive }) => (isActive ? 'active' : undefined)}>
            New Project
          </NavLink>
          <NavLink to="/settings" className={({ isActive }) => (isActive ? 'active' : undefined)}>
            Settings
          </NavLink>
        </nav>
        <span className="badge">no OAuth · public GitHub only</span>
      </header>
      <main className="main">{children}</main>
    </div>
  )
}

function statusClass(s) {
  if (!s) return ''
  return s.toLowerCase()
}

function ProjectsPage() {
  const [projects, setProjects] = useState([])
  const [err, setErr] = useState('')
  const [loading, setLoading] = useState(true)

  const load = useCallback(async () => {
    try {
      const data = await api.listProjects()
      setProjects(data.projects || [])
      setErr('')
    } catch (e) {
      setErr(e.message)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    load()
    const t = setInterval(load, 8000)
    return () => clearInterval(t)
  }, [load])

  return (
    <Shell>
      <div className="hero">
        <h1>Projects</h1>
        <p>
          Link any <strong>public</strong> GitHub repo with <code>owner/repo</code> or a full URL.
          Flare clones over HTTPS (no API keys), polls for new commits, builds, and serves previews.
        </p>
        <div className="row">
          <Link to="/new"><button className="primary" type="button">New Project</button></Link>
          <button type="button" onClick={load}>Refresh</button>
        </div>
      </div>
      <div className="spacer" />
      {err && <div className="error-box">{err} — is the API running on :8080?</div>}
      {loading && !projects.length && <p className="muted">Loading…</p>}
      {!loading && !projects.length && !err && (
        <p className="muted">No projects yet. Try linking <code>mdn/beginner-html-site</code>.</p>
      )}
      <div className="grid">
        {projects.map((p) => (
          <Link key={p.id} to={`/projects/${p.id}`} style={{ textDecoration: 'none', color: 'inherit' }}>
            <article className="card">
              <h3>{p.name}</h3>
              <div className="meta">{p.owner_repo}</div>
              <div className="row" style={{ marginTop: '0.65rem' }}>
                {p.framework && <span className="pill">{p.framework}</span>}
                <span className="pill">{p.default_branch}</span>
                {p.poll_enabled && <span className="pill">auto-deploy</span>}
              </div>
            </article>
          </Link>
        ))}
      </div>
    </Shell>
  )
}

function NewProjectPage() {
  const nav = useNavigate()
  const [github, setGithub] = useState('')
  const [name, setName] = useState('')
  const [branch, setBranch] = useState('main')
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState('')

  async function onSubmit(e) {
    e.preventDefault()
    setBusy(true)
    setErr('')
    try {
      const p = await api.createProject({
        github: github.trim(),
        name: name.trim() || undefined,
        branch: branch.trim() || 'main',
      })
      nav(`/projects/${p.id}`)
    } catch (ex) {
      setErr(ex.message)
    } finally {
      setBusy(false)
    }
  }

  return (
    <Shell>
      <div className="hero">
        <h1>Link a public repo</h1>
        <p>Examples: <code>vercel/next.js</code>, <code>https://github.com/withastro/astro</code></p>
      </div>
      <form className="form card" onSubmit={onSubmit}>
        <label>
          GitHub (public)
          <input
            required
            placeholder="owner/repo or https://github.com/owner/repo"
            value={github}
            onChange={(e) => setGithub(e.target.value)}
          />
        </label>
        <label>
          Project name (optional)
          <input placeholder="My site" value={name} onChange={(e) => setName(e.target.value)} />
        </label>
        <label>
          Branch
          <input value={branch} onChange={(e) => setBranch(e.target.value)} />
        </label>
        {err && <div className="error-box">{err}</div>}
        <div className="row">
          <button className="primary" type="submit" disabled={busy}>
            {busy ? 'Cloning…' : 'Create & deploy'}
          </button>
          <button type="button" onClick={() => nav('/')}>Cancel</button>
        </div>
      </form>
    </Shell>
  )
}

function ProjectDetailPage() {
  const { id } = useParams()
  const nav = useNavigate()
  const [project, setProject] = useState(null)
  const [deployments, setDeployments] = useState([])
  const [env, setEnv] = useState([])
  const [ek, setEk] = useState('')
  const [ev, setEv] = useState('')
  const [err, setErr] = useState('')
  const [logsFor, setLogsFor] = useState(null)
  const [logs, setLogs] = useState([])

  const load = useCallback(async () => {
    try {
      const [p, d, e] = await Promise.all([
        api.getProject(id),
        api.listDeployments(id),
        api.listEnv(id),
      ])
      setProject(p)
      setDeployments(d.deployments || [])
      setEnv(e.env || [])
      setErr('')
    } catch (ex) {
      setErr(ex.message)
    }
  }, [id])

  useEffect(() => {
    load()
    const t = setInterval(load, 4000)
    return () => clearInterval(t)
  }, [load])

  useEffect(() => {
    if (!logsFor) return undefined
    let cancelled = false
    async function pull() {
      try {
        const data = await api.getLogs(logsFor)
        if (!cancelled) setLogs(data.logs || [])
      } catch {
        /* ignore */
      }
    }
    pull()
    const t = setInterval(pull, 2000)
    return () => {
      cancelled = true
      clearInterval(t)
    }
  }, [logsFor])

  async function redeploy() {
    try {
      await api.deploy(id)
      await load()
    } catch (ex) {
      setErr(ex.message)
    }
  }

  async function remove() {
    if (!window.confirm('Delete this project?')) return
    await api.deleteProject(id)
    nav('/')
  }

  async function addEnv(e) {
    e.preventDefault()
    await api.upsertEnv(id, ek, ev)
    setEk('')
    setEv('')
    load()
  }

  if (!project && !err) {
    return (
      <Shell>
        <p className="muted">Loading…</p>
      </Shell>
    )
  }

  return (
    <Shell>
      {err && <div className="error-box">{err}</div>}
      {project && (
        <>
          <div className="hero">
            <h1>{project.name}</h1>
            <p>
              <a href={project.github_url} target="_blank" rel="noreferrer">{project.owner_repo}</a>
              {' · '}
              branch <code>{project.default_branch}</code>
              {project.framework && <> · <span className="pill">{project.framework}</span></>}
            </p>
            <div className="row">
              <button className="primary" type="button" onClick={redeploy}>Deploy now</button>
              <button type="button" className="danger" onClick={remove}>Delete</button>
            </div>
          </div>

          <h2 style={{ fontSize: '1.1rem', marginTop: '1.5rem' }}>Deployments</h2>
          <div className="card" style={{ padding: 0, overflow: 'hidden' }}>
            <table className="table">
              <thead>
                <tr>
                  <th>Status</th>
                  <th>Commit</th>
                  <th>Message</th>
                  <th>Changed files</th>
                  <th>Preview</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {deployments.map((d) => (
                  <tr key={d.id}>
                    <td><span className={`status ${statusClass(d.status)}`}>{d.status}</span></td>
                    <td><code>{d.commit_sha?.slice(0, 7)}</code></td>
                    <td className="muted">{d.commit_message || '—'}</td>
                    <td className="muted" style={{ maxWidth: 180, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                      {d.changed_files ? d.changed_files.split('\n').length + ' files' : '—'}
                    </td>
                    <td>
                      {d.url_path && d.status === 'ready' ? (
                        <a href={d.url_path} target="_blank" rel="noreferrer">Open</a>
                      ) : '—'}
                    </td>
                    <td>
                      <button type="button" onClick={() => setLogsFor(d.id)}>Logs</button>
                    </td>
                  </tr>
                ))}
                {!deployments.length && (
                  <tr><td colSpan={6} className="muted">No deployments yet</td></tr>
                )}
              </tbody>
            </table>
          </div>

          {logsFor && (
            <>
              <div className="row" style={{ marginTop: '1rem' }}>
                <h2 style={{ fontSize: '1.1rem', margin: 0 }}>Build logs</h2>
                <button type="button" onClick={() => setLogsFor(null)}>Close</button>
              </div>
              <div className="logs">
                {logs.map((l) => (
                  <div key={l.id}>{l.line}</div>
                ))}
                {!logs.length && <span className="muted">Waiting for logs…</span>}
              </div>
              {deployments.find((d) => d.id === logsFor)?.changed_files && (
                <>
                  <h3 style={{ fontSize: '0.95rem' }}>Changed files</h3>
                  <div className="logs">
                    {deployments.find((d) => d.id === logsFor).changed_files}
                  </div>
                </>
              )}
            </>
          )}

          <h2 style={{ fontSize: '1.1rem', marginTop: '1.5rem' }}>Environment variables</h2>
          <form className="row form" style={{ maxWidth: '100%' }} onSubmit={addEnv}>
            <input placeholder="KEY" value={ek} onChange={(e) => setEk(e.target.value)} required style={{ maxWidth: 180 }} />
            <input placeholder="value" value={ev} onChange={(e) => setEv(e.target.value)} required style={{ flex: 1, minWidth: 160 }} />
            <button className="primary" type="submit">Add</button>
          </form>
          <ul className="muted">
            {env.map((v) => (
              <li key={v.id}>
                <code>{v.key}</code>=••••••{' '}
                <button
                  type="button"
                  onClick={async () => {
                    await api.deleteEnv(id, v.key)
                    load()
                  }}
                >
                  remove
                </button>
              </li>
            ))}
          </ul>
        </>
      )}
    </Shell>
  )
}

function SettingsPage() {
  const [pollSecs, setPollSecs] = useState('60')
  const [err, setErr] = useState('')
  const [ok, setOk] = useState('')
  const [busy, setBusy] = useState(false)
  const [loading, setLoading] = useState(true)

  const load = useCallback(async () => {
    try {
      const data = await api.getSettings()
      const v = data.settings?.poll_interval_secs
      if (v != null) setPollSecs(String(v))
      setErr('')
    } catch (e) {
      setErr(e.message)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    load()
  }, [load])

  async function onSubmit(e) {
    e.preventDefault()
    setBusy(true)
    setErr('')
    setOk('')
    const n = Number(pollSecs)
    if (!Number.isFinite(n) || n < 5) {
      setErr('poll_interval_secs must be a number ≥ 5')
      setBusy(false)
      return
    }
    try {
      const data = await api.updateSettings({ poll_interval_secs: Math.floor(n) })
      setPollSecs(String(data.settings?.poll_interval_secs ?? n))
      setOk('Settings saved. Poller picks up the new interval on the next sleep.')
    } catch (ex) {
      setErr(ex.message)
    } finally {
      setBusy(false)
    }
  }

  return (
    <Shell>
      <div className="hero">
        <h1>Settings</h1>
        <p>
          Platform configuration stored in SQLite. No OAuth or API keys — Flare only talks to
          public GitHub over HTTPS.
        </p>
      </div>
      {loading ? (
        <p className="muted">Loading…</p>
      ) : (
        <form className="form card" onSubmit={onSubmit}>
          <label>
            Poll interval (seconds)
            <input
              type="number"
              min={5}
              step={1}
              value={pollSecs}
              onChange={(e) => setPollSecs(e.target.value)}
              required
            />
          </label>
          <p className="muted" style={{ margin: 0, fontSize: '0.85rem' }}>
            How often Flare checks linked public remotes for new commits (minimum 5s). Default 60.
          </p>
          {err && <div className="error-box">{err}</div>}
          {ok && <div className="ok-box">{ok}</div>}
          <div className="row">
            <button className="primary" type="submit" disabled={busy}>
              {busy ? 'Saving…' : 'Save settings'}
            </button>
            <button type="button" onClick={load}>Reload</button>
          </div>
        </form>
      )}
    </Shell>
  )
}

export default function App() {
  return (
    <Routes>
      <Route path="/" element={<ProjectsPage />} />
      <Route path="/new" element={<NewProjectPage />} />
      <Route path="/projects/:id" element={<ProjectDetailPage />} />
      <Route path="/settings" element={<SettingsPage />} />
    </Routes>
  )
}
