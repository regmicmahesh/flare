const BASE = ''

async function req(path, opts = {}) {
  const res = await fetch(`${BASE}${path}`, {
    headers: { 'Content-Type': 'application/json', ...(opts.headers || {}) },
    ...opts,
  })
  if (res.status === 204) return null
  const text = await res.text()
  let data
  try {
    data = text ? JSON.parse(text) : null
  } catch {
    data = text
  }
  if (!res.ok) {
    const msg = typeof data === 'string' ? data : data?.message || res.statusText
    throw new Error(msg || `HTTP ${res.status}`)
  }
  return data
}

export const api = {
  health: () => req('/api/health'),
  listProjects: () => req('/api/projects'),
  getProject: (id) => req(`/api/projects/${id}`),
  createProject: (body) =>
    req('/api/projects', { method: 'POST', body: JSON.stringify(body) }),
  updateProject: (id, body) =>
    req(`/api/projects/${id}`, { method: 'PATCH', body: JSON.stringify(body) }),
  deleteProject: (id) => req(`/api/projects/${id}`, { method: 'DELETE' }),
  deploy: (id, body) =>
    req(`/api/projects/${id}/deploy`, {
      method: 'POST',
      body: JSON.stringify(body || {}),
    }),
  rollback: (id, deployment_id) =>
    req(`/api/projects/${id}/rollback`, {
      method: 'POST',
      body: JSON.stringify(deployment_id ? { deployment_id } : {}),
    }),
  projectStats: (id) => req(`/api/projects/${id}/stats`),
  deploymentStats: (id) => req(`/api/deployments/${id}/stats`),
  deploymentDiff: (a, b) => req(`/api/deployments/${a}/diff/${b}`),
  promote: (id, deployment_id) =>
    req(`/api/projects/${id}/promote`, {
      method: 'POST',
      body: JSON.stringify({ deployment_id }),
    }),
  listDeployments: (id) => req(`/api/projects/${id}/deployments`),
  listCommits: (id, limit = 20) =>
    req(`/api/projects/${id}/commits?limit=${limit}`),
  listActivity: (id) => req(`/api/projects/${id}/activity`),
  getDeployment: (id) => req(`/api/deployments/${id}`),
  getLogs: (id) => req(`/api/deployments/${id}/logs`),
  listEnv: (id) => req(`/api/projects/${id}/env`),
  upsertEnv: (id, key, value) =>
    req(`/api/projects/${id}/env`, {
      method: 'POST',
      body: JSON.stringify({ key, value }),
    }),
  deleteEnv: (id, key) =>
    req(`/api/projects/${id}/env/${encodeURIComponent(key)}`, { method: 'DELETE' }),
  getSettings: () => req('/api/settings'),
  updateSettings: (body) =>
    req('/api/settings', { method: 'PATCH', body: JSON.stringify(body) }),
  cancelDeployment: (id) =>
    req(`/api/deployments/${id}/cancel`, { method: 'POST' }),
  listWebhooks: (id) => req(`/api/projects/${id}/webhooks`),
  createWebhook: (id, body) =>
    req(`/api/projects/${id}/webhooks`, {
      method: 'POST',
      body: JSON.stringify(body),
    }),
  deleteWebhook: (id, webhookId) =>
    req(`/api/projects/${id}/webhooks/${webhookId}`, { method: 'DELETE' }),
  listDomains: (id) => req(`/api/projects/${id}/domains`),
  createDomain: (id, body) =>
    req(`/api/projects/${id}/domains`, {
      method: 'POST',
      body: JSON.stringify(body),
    }),
  deleteDomain: (id, domainId) =>
    req(`/api/projects/${id}/domains/${domainId}`, { method: 'DELETE' }),
}
