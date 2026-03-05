import { Hono } from 'hono';
import { html } from 'hono/html';
import { serveStatic } from 'hono/deno';

const app = new Hono();

// Helper to generate HMAC Token
async function generateToken(ip) {
  const secretStr = Deno.env.get('TOKEN_SECRET');
  if (!secretStr) return 'no-token';

  const encoder = new TextEncoder();
  const keyMaterial = await crypto.subtle.importKey(
    'raw',
    encoder.encode(secretStr),
    { name: 'HMAC', hash: 'SHA-256' },
    false,
    ['sign']
  );

  const hour = Math.floor(Date.now() / 3600000);
  const data = encoder.encode(`${hour}:${ip || 'unknown'}`);

  const signature = await crypto.subtle.sign('HMAC', keyMaterial, data);

  // Convert ArrayBuffer to hex string
  const hashArray = Array.from(new Uint8Array(signature));
  const hashHex = hashArray.map(b => b.toString(16).padStart(2, '0')).join('');
  return hashHex;
}

// Serve static files
app.use('/static/*', serveStatic({ root: './' }));

// Token Refresh
app.post('/api/token', async (c) => {
  const origin = c.req.header('Origin');
  if (!origin) return c.json({ error: 'Forbidden' }, 403);

  const ip = c.req.header('cf-connecting-ip');
  const token = await generateToken(ip);
  return c.json({ token });
});

// Manifest Proxy
app.get('/api/manifest', async (c) => {
  const r2PublicUrl = Deno.env.get('R2_PUBLIC_URL') || 'http://localhost:8080/manifest.json';
  try {
    const headers = new Headers();
    const ifNoneMatch = c.req.header('If-None-Match');
    if (ifNoneMatch) {
      headers.set('If-None-Match', ifNoneMatch);
    }

    const response = await fetch(r2PublicUrl, { headers });

    if (response.status === 304) {
      return c.body(null, 304);
    }

    if (!response.ok) throw new Error('Bad R2 Response');
    const manifest = await response.json();
    return c.json(manifest, 200, {
      'Cache-Control': 's-maxage=5, stale-while-revalidate=2'
    });
  } catch (err) {
    console.error('Manifest proxy error:', err);
    return c.json({ live: false }, 200, {
      'Cache-Control': 's-maxage=5, stale-while-revalidate=2'
    });
  }
});

// SSR Route
app.get('/', async (c) => {
  const ip = c.req.header('cf-connecting-ip');
  let token = 'no-token';
  let live = 'true';

  try {
    token = await generateToken(ip);

    // Check if manifest is valid to set data-live
    const r2PublicUrl = Deno.env.get('R2_PUBLIC_URL') || 'http://localhost:8080/manifest.json';
    const response = await fetch(r2PublicUrl);
    if (!response.ok) {
      const txt = await response.text();
      throw new Error(`Bad R2 Response: ${response.status} ${txt}`);
    }
    const manifest = await response.json();
    live = manifest.live ? 'true' : 'false';
  } catch (err) {
    console.error('Failed to fetch manifest or token during SSR:', err);
    live = 'false';
  }

  // Base R2 URL for fetching chunks directly
  let r2BaseUrl = Deno.env.get('PUBLIC_R2_URL') || 'http://localhost:8080';

  // Provide Events URL for client
  let eventsUrl = Deno.env.get('PUBLIC_EVENTS_URL') || 'http://localhost:8080/events';

  return c.html(html`
    <!DOCTYPE html>
    <html lang="en">
    <head>
      <meta charset="UTF-8">
      <meta name="viewport" content="width=device-width, initial-scale=1.0">
      <title>Lossless Radio Player</title>
      <script type="module" src="/static/player.js"></script>
    </head>
    <body>
      <radio-player data-token="${token}" data-live="${live}" data-r2-url="${r2BaseUrl}" data-events-url="${eventsUrl}"></radio-player>
    </body>
    </html>
  `);
});

Deno.serve({ port: 3000 }, app.fetch);
