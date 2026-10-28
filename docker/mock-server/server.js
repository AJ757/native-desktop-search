// Docker mock server for cloud connectors (Microsoft 365, OneDrive, Notion)
// Zero external dependencies - runs on standard Node.js http module.

const http = require("http");
const url = require("url");

const PORT = process.env.PORT || 8080;

// In-memory state for tracking calls and delta syncs
let msDeltaCallCount = 0;
let notionCallCount = 0;

const server = http.createServer((req, res) => {
  const parsedUrl = url.parse(req.url, true);
  const pathname = parsedUrl.pathname;
  const method = req.method.toUpperCase();

  // Helper for JSON responses
  const sendJson = (status, data) => {
    res.writeHead(status, { "Content-Type": "application/json" });
    res.end(JSON.stringify(data));
  };

  // Helper for text responses
  const sendText = (status, text, contentType = "text/plain") => {
    res.writeHead(status, { "Content-Type": contentType });
    res.end(text);
  };

  // Log requests
  console.log(`[MockServer] ${method} ${pathname}`);

  // 1. Health check
  if (pathname === "/health" || pathname === "/") {
    return sendJson(200, { status: "healthy", timestamp: new Date().toISOString() });
  }

  // 2. Microsoft OAuth endpoints
  // Token exchange
  if (
    (pathname === "/microsoft/token" || pathname === "/common/oauth2/v2.0/token") &&
    method === "POST"
  ) {
    let body = "";
    req.on("data", (chunk) => (body += chunk));
    req.on("end", () => {
      console.log("[MockServer] Microsoft OAuth Token requested with body:", body);
      return sendJson(200, {
        token_type: "Bearer",
        scope: "offline_access Files.Read.All User.Read",
        expires_in: 3600,
        access_token: "mock_ms_access_token_12345",
        refresh_token: "mock_ms_refresh_token_67890",
        id_token: "mock_ms_id_token",
      });
    });
    return;
  }

  // Authorize redirect simulation
  if (pathname === "/microsoft/authorize" || pathname === "/common/oauth2/v2.0/authorize") {
    const redirectUri = parsedUrl.query.redirect_uri || "http://127.0.0.1:53682/callback";
    const state = parsedUrl.query.state || "state123";
    const code = "mock_auth_code_ms_abc";
    const target = `${redirectUri}?code=${code}&state=${state}`;
    res.writeHead(302, { Location: target });
    return res.end();
  }

  // 3. Microsoft Graph API Endpoints (Base URL /graph)
  // User profile
  if (pathname === "/graph/v1.0/me" && method === "GET") {
    return sendJson(200, {
      id: "ms-user-001",
      displayName: "DevOps Engineer",
      mail: "engineer@company.com",
      userPrincipalName: "engineer@company.com",
    });
  }

  // OneDrive Delta API
  if (pathname === "/graph/v1.0/me/drive/root/delta" && method === "GET") {
    const token = parsedUrl.query.token;
    msDeltaCallCount++;

    if (!token) {
      // Initial crawl: returns initial files and a delta link
      return sendJson(200, {
        "@odata.context": "https://graph.microsoft.com/v1.0/$metadata#Collection(driveItem)",
        "@odata.deltaLink": `http://127.0.0.1:${PORT}/graph/v1.0/me/drive/root/delta?token=delta_v1`,
        value: [
          {
            id: "onedrive_file_1",
            name: "Cloud_Architecture_Guide.md",
            size: 2048,
            lastModifiedDateTime: "2026-10-25T12:00:00Z",
            file: { mimeType: "text/markdown" },
            "@microsoft.graph.downloadUrl": `http://127.0.0.1:${PORT}/graph/download/onedrive_file_1`,
          },
          {
            id: "onedrive_file_2",
            name: "DevOps_Pipeline_Plan.docx",
            size: 4096,
            lastModifiedDateTime: "2026-10-26T14:30:00Z",
            file: {
              mimeType: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            },
            "@microsoft.graph.downloadUrl": `http://127.0.0.1:${PORT}/graph/download/onedrive_file_2`,
          },
        ],
      });
    } else if (token === "delta_v1") {
      // Incremental sync: 1 modified file, 1 deleted file
      return sendJson(200, {
        "@odata.context": "https://graph.microsoft.com/v1.0/$metadata#Collection(driveItem)",
        "@odata.deltaLink": `http://127.0.0.1:${PORT}/graph/v1.0/me/drive/root/delta?token=delta_v2`,
        value: [
          {
            id: "onedrive_file_1",
            name: "Cloud_Architecture_Guide.md",
            size: 2500,
            lastModifiedDateTime: "2026-10-28T09:15:00Z",
            file: { mimeType: "text/markdown" },
            "@microsoft.graph.downloadUrl": `http://127.0.0.1:${PORT}/graph/download/onedrive_file_1`,
          },
          {
            id: "onedrive_file_2",
            name: "DevOps_Pipeline_Plan.docx",
            deleted: { state: "deleted" },
          },
        ],
      });
    } else {
      // Subsequent delta: no changes
      return sendJson(200, {
        "@odata.context": "https://graph.microsoft.com/v1.0/$metadata#Collection(driveItem)",
        "@odata.deltaLink": `http://127.0.0.1:${PORT}/graph/v1.0/me/drive/root/delta?token=delta_v2`,
        value: [],
      });
    }
  }

  // File download mock
  if (pathname.startsWith("/graph/download/")) {
    const fileId = pathname.replace("/graph/download/", "");
    return sendText(
      200,
      `# Content for ${fileId}\nThis is mock OneDrive file content indexed for desktop search.`,
    );
  }

  // 4. Notion OAuth & API Endpoints (Base URL /notion)
  if (pathname === "/notion/v1/oauth/token" && method === "POST") {
    let body = "";
    req.on("data", (chunk) => (body += chunk));
    req.on("end", () => {
      console.log("[MockServer] Notion OAuth Token requested:", body);
      return sendJson(200, {
        access_token: "mock_notion_access_token_abc123",
        token_type: "bearer",
        bot_id: "bot_notion_456",
        workspace_name: "Engineering Wiki",
        workspace_icon: "🚀",
        workspace_id: "notion_ws_999",
        owner: {
          type: "user",
          user: {
            object: "user",
            id: "user_notion_1",
            name: "DevOps Lead",
          },
        },
      });
    });
    return;
  }

  // Notion search endpoint (pages/databases)
  if (pathname === "/notion/v1/search" && method === "POST") {
    let body = "";
    req.on("data", (chunk) => (body += chunk));
    req.on("end", () => {
      notionCallCount++;
      return sendJson(200, {
        object: "list",
        results: [
          {
            object: "page",
            id: "notion_page_001",
            created_time: "2026-10-21T10:00:00.000Z",
            last_edited_time: "2026-10-27T16:45:00.000Z",
            archived: false,
            properties: {
              title: {
                id: "title",
                type: "title",
                title: [
                  {
                    type: "text",
                    text: { content: "Kubernetes Deployment Strategy" },
                    plain_text: "Kubernetes Deployment Strategy",
                  },
                ],
              },
            },
            url: "https://notion.so/Kubernetes-Deployment-Strategy-001",
          },
          {
            object: "page",
            id: "notion_page_002",
            created_time: "2026-10-22T11:00:00.000Z",
            last_edited_time: "2026-10-28T08:00:00.000Z",
            archived: false,
            properties: {
              title: {
                id: "title",
                type: "title",
                title: [
                  {
                    type: "text",
                    text: { content: "CI/CD Pipeline Security Checklist" },
                    plain_text: "CI/CD Pipeline Security Checklist",
                  },
                ],
              },
            },
            url: "https://notion.so/CICD-Pipeline-Security-Checklist-002",
          },
        ],
        next_cursor: null,
        has_more: false,
      });
    });
    return;
  }

  // Notion page blocks children (page content)
  if (
    pathname.startsWith("/notion/v1/blocks/") &&
    pathname.endsWith("/children") &&
    method === "GET"
  ) {
    return sendJson(200, {
      object: "list",
      results: [
        {
          object: "block",
          id: "block_1",
          type: "paragraph",
          paragraph: {
            rich_text: [
              {
                type: "text",
                text: {
                  content: "Overview of Kubernetes multi-stage cluster deployment with Helm.",
                },
                plain_text: "Overview of Kubernetes multi-stage cluster deployment with Helm.",
              },
            ],
          },
        },
      ],
      next_cursor: null,
      has_more: false,
    });
  }

  // Fallback
  return sendJson(404, { error: "Not Found", pathname });
});

server.listen(PORT, "0.0.0.0", () => {
  console.log(`[MockServer] Cloud connectors mock server listening on port ${PORT}`);
});
