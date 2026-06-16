import * as BunnySDK from "@bunny.net/edgescript-sdk";

// Embedded copy of site/install.sh (source of truth).
// Must stay in sync — CI checks for drift on every PR and push (site.yml).
const INSTALL_SCRIPT = ``;

const LANDING_PAGE = ``;

const LLMS_TXT = ``;

BunnySDK.net.http.serve(async (request: Request): Promise<Response> => {
  const url = new URL(request.url);

  // Redirect purple-ssh.com → getpurple.sh
  const host = request.headers.get("host") || "";
  if (host === "purple-ssh.com" || host === "www.purple-ssh.com" || host === "www.getpurple.sh") {
    return Response.redirect(`https://getpurple.sh${url.pathname}${url.search}`, 301);
  }
  if (url.pathname === "/llms.txt") {
    return new Response(LLMS_TXT, {
      headers: {
        "content-type": "text/plain; charset=utf-8",
        "cache-control": "public, max-age=3600",
      },
    });
  }
  // Real robots.txt so AI crawlers and search bots get a parseable allow
  // policy (not the HTML page) and can discover llms.txt.
  if (url.pathname === "/robots.txt") {
    return new Response(
      "User-agent: *\nAllow: /\n\n# AI/LLM overview: https://getpurple.sh/llms.txt\n",
      {
        headers: {
          "content-type": "text/plain; charset=utf-8",
          "cache-control": "public, max-age=3600",
        },
      },
    );
  }

  const ua = (request.headers.get("user-agent") || "").toLowerCase();
  const isCli =
    ua.startsWith("curl") ||
    ua.startsWith("wget") ||
    ua.startsWith("fetch") ||
    ua.startsWith("httpie");

  if (isCli) {
    return new Response(INSTALL_SCRIPT, {
      headers: {
        "content-type": "text/plain; charset=utf-8",
        "cache-control": "public, max-age=300",
      },
    });
  }

  return new Response(LANDING_PAGE, {
    headers: {
      "content-type": "text/html; charset=utf-8",
      "cache-control": "public, max-age=3600",
    },
  });
});
