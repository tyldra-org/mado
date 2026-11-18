// Canned answers to the queries screens make, so the gallery shows content.
export function answers(type: string): unknown {
  switch (type) {
    case "control.clients":
      return [{ id: "c1", name: "mado", scopes: ["read", "act", "tasks"], revoked: false }];
    case "ai.skill.list":
      return [
        { name: "research", description: "Answer a question from several sources", builtin: true },
      ];
    case "nav.suggest":
      return { requestId: 1, rows: [] };
    case "history.topSites":
      return [
        { url: "https://mail.example/", title: "Mail" },
        { url: "https://news.example/", title: "News" },
        { url: "https://github.com/", title: "GitHub" },
        { url: "https://docs.example/", title: "Docs" },
      ];
    default:
      return null;
  }
}
