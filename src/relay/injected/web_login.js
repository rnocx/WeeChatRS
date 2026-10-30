(() => {
  if (window !== window.top || location.origin !== 'https://injected-irc.com') return;
  let sent = false;
  let checking = false;
  // Observe the official app's completed session. Leave the site's forms,
  // CAPTCHA, WebAuthn, fetch, and storage behavior untouched.
  const check = async () => {
    if (sent || checking) return;
    const token = localStorage.getItem('injected_token');
    if (!token || token.length > 16384) return;
    checking = true;
    try {
      const response = await fetch('/api/me', {
        headers: { Authorization: `Bearer ${token}` }, redirect: 'error'
      });
      if (!response.ok || sent) return;
      const user = await response.json();
      if (!user || typeof user.username !== 'string') return;
      if (localStorage.getItem('injected_token') !== token) return;
      sent = true;
      window.ipc.postMessage(JSON.stringify({ token }));
    } catch (_) {
      // Keep the official login usable if the connection briefly drops.
    } finally {
      checking = false;
    }
  };
  window.addEventListener('DOMContentLoaded', check, { once: true });
  window.addEventListener('storage', check);
  setInterval(check, 750);
})();
