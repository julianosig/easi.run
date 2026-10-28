<script lang="ts">
  import Rich from '../Rich.svelte';
  /** The routes' needs, by the message that names each. */
  const NEEDS: Record<string, string> = { none: 'none', Origin: 'origin', 'Origin + login': 'origin-login', login: 'login', 'login + CSRF': 'login-csrf' };
  const routes: [string, string, string, string][] = [
    ['GET', '/api/status', 'api.route.get-api-status', 'none'],
    ['POST', '/api/enroll', 'api.route.post-api-enroll', 'Origin'],
    ['POST', '/api/login', 'api.route.post-api-login', 'Origin'],
    ['POST', '/api/login/code', 'api.route.post-api-login-code', 'Origin'],
    ['POST', '/api/login/passkey', 'api.route.post-api-login-passkey', 'Origin'],
    ['POST', '/api/login/passkey/finish', 'api.route.post-api-login-passkey-finish', 'Origin'],
    ['POST', '/api/demo/switch', 'api.route.post-api-demo-switch', 'login + CSRF'],
    ['POST', '/api/logout', 'api.route.post-api-logout', 'login + CSRF'],
    ['GET', '/api/me', 'api.route.get-api-me', 'login'],
    ['GET', '/api/egresses', 'api.route.get-api-egresses', 'login'],
    ['GET', '/api/egresses/options', 'api.route.get-api-egresses-options', 'login'],
    ['POST', '/api/egresses', 'api.route.post-api-egresses', 'login + CSRF'],
    ['PATCH', '/api/egresses/{id}', 'api.route.patch-api-egresses-id', 'login + CSRF'],
    ['DELETE', '/api/egresses/{id}', 'api.route.delete-api-egresses-id', 'login + CSRF'],
    ['POST', '/api/egresses/check', 'api.route.post-api-egresses-check', 'login + CSRF'],
    ['POST', '/api/egresses/{id}/check', 'api.route.post-api-egresses-id-check', 'login + CSRF'],
    ['GET', '/api/sessions', 'api.route.get-api-sessions', 'login'],
    ['POST', '/api/sessions', 'api.route.post-api-sessions', 'login + CSRF'],
    ['DELETE', '/api/sessions/{id}', 'api.route.delete-api-sessions-id', 'login + CSRF'],
    ['POST', '/api/sessions/{id}/control', 'api.route.post-api-sessions-id-control', 'login + CSRF'],
    ['POST', '/api/sessions/{id}/release', 'api.route.post-api-sessions-id-release', 'login + CSRF'],
    ['PUT', '/api/sessions/{id}/settings', 'api.route.put-api-sessions-id-settings', 'login + CSRF'],
    ['PUT', '/api/sessions/{id}/network', 'api.route.put-api-sessions-id-network', 'login + CSRF'],
    ['POST', '/api/sessions/{id}/agent', 'api.route.post-api-sessions-id-agent', 'login + CSRF'],
    ['GET', '/api/sessions/{id}/stats', 'api.route.get-api-sessions-id-stats', 'login'],
    ['GET', '/api/sessions/{id}/files/{ticket}', 'api.route.get-api-sessions-id-files-ticket', 'login'],
    ['GET', '/ws/sessions/{id}?viewer=…', 'api.route.get-ws-sessions-id-viewer', 'Origin + login'],
    ['GET', '/api/tokens', 'api.route.get-api-tokens', 'login'],
    ['POST', '/api/tokens', 'api.route.post-api-tokens', 'login + CSRF'],
    ['DELETE', '/api/tokens/{id}', 'api.route.delete-api-tokens-id', 'login + CSRF'],
    ['GET', '/api/profiles', 'api.route.get-api-profiles', 'login'],
    ['POST', '/api/profiles', 'api.route.post-api-profiles', 'login + CSRF'],
    ['PUT', '/api/profiles/{id}', 'api.route.put-api-profiles-id', 'login + CSRF'],
    ['DELETE', '/api/profiles/{id}', 'api.route.delete-api-profiles-id', 'login + CSRF'],
    ['POST', '/api/profiles/unlock', 'api.route.post-api-profiles-unlock', 'login + CSRF'],
    ['GET', '/api/security', 'api.route.get-api-security', 'login'],
    ['POST', '/api/security/app', 'api.route.post-api-security-app', 'login + CSRF'],
    ['POST', '/api/security/app/confirm', 'api.route.post-api-security-app-confirm', 'login + CSRF'],
    ['POST', '/api/security/app/remove', 'api.route.post-api-security-app-remove', 'login + CSRF'],
    ['POST', '/api/security/passkeys', 'api.route.post-api-security-passkeys', 'login + CSRF'],
    ['POST', '/api/security/passkeys/finish', 'api.route.post-api-security-passkeys-finish', 'login + CSRF'],
    ['POST', '/api/security/passkeys/{id}/remove', 'api.route.post-api-security-passkeys-id-remove', 'login + CSRF'],
    ['POST', '/api/security/recovery', 'api.route.post-api-security-recovery', 'login + CSRF'],
  ];
</script>

<h1 tabindex="-1"><Rich message="api.title" /></h1>
<p class="lead"><Rich message="api.intro.1" /></p>

<h2><Rich message="api.authentication.title" /></h2>
<ul>
  <li><Rich message="api.authentication.1" /></li>
  <li><Rich message="api.authentication.2" /></li>
  <li><Rich message="api.authentication.3" /></li>
  <li><Rich message="api.authentication.4" /></li>
  <li><Rich message="api.authentication.5" /></li>
  <li><Rich message="api.authentication.6" /></li>
</ul>

<p><Rich message="api.authentication.7" /></p>

<h2><Rich message="api.routes.title" /></h2>
<table>
  <thead><tr><th><Rich message="api.routes.1" /></th><th><Rich message="api.routes.2" /></th><th><Rich message="api.routes.3" /></th></tr></thead>
  <tbody>
    {#each routes as [method, path, what, needs] (method + path)}
      <tr><td><code>{method} {path}</code></td><td><Rich message={what} /></td><td class="whitespace-nowrap text-muted"><Rich message={`api.needs.${NEEDS[needs]}`} /></td></tr>
    {/each}
  </tbody>
</table>

<h2><Rich message="api.encoding-settings.title" /></h2>
<table>
  <thead><tr><th><Rich message="api.encoding-settings.1" /></th><th><Rich message="api.encoding-settings.2" /></th></tr></thead>
  <tbody>
    <tr><td><Rich message="api.encoding-settings.3" /></td><td><Rich message="api.encoding-settings.4" /></td></tr>
    <tr><td><Rich message="api.encoding-settings.5" /></td><td><Rich message="api.encoding-settings.6" /></td></tr>
    <tr><td><Rich message="api.encoding-settings.7" /></td><td><Rich message="api.encoding-settings.8" /></td></tr>
  </tbody>
</table>
