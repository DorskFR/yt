"""Minimal fake YouTrack REST API for docs/demo.tape. Invented data only."""

import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import unquote, urlparse

DAY = 86_400_000
T0 = 1_793_000_000_000


def cf(name, value):
    return {"name": name, "value": value}


def issue(n, summary, state, prio, who=None, tags=(), desc="", reporter="bob", age=3):
    return {
        "idReadable": f"DEMO-{n}",
        "summary": summary,
        "description": desc,
        "created": T0 - age * DAY,
        "updated": T0 - DAY,
        "reporter": {"login": reporter},
        "tags": [{"name": t} for t in tags],
        "customFields": [
            cf("State", {"name": state}),
            cf("Priority", {"name": prio}),
            cf("Assignee", {"login": who} if who else None),
        ],
    }


ISSUES = [
    issue(42, "Login page times out on slow networks", "In Progress", "Critical", "alice",
          ("auth",), "Requests over 3G exceed the 10 s gateway timeout.\nRepro: throttle to Slow 3G, submit the form.", age=6),
    issue(47, "Password reset email has a broken link", "Open", "Major", "bob", ("auth", "email")),
    issue(51, "Export to CSV drops the last row", "Open", "Normal", None, ("export",)),
    issue(53, "Dark mode toggle resets after reload", "Open", "Minor", "carol"),
    issue(55, "Search results ignore the date filter", "In Review", "Major", "alice"),
]
LINKS = {
    "DEMO-42": [{
        "direction": "OUTWARD",
        "linkType": {"name": "Relates", "sourceToTarget": "relates to", "targetToSource": ""},
        "issues": [{"idReadable": "DEMO-47", "summary": "Password reset email has a broken link"}],
    }],
}
COMMENTS = {
    "DEMO-42": [
        {"id": "4-311", "author": {"login": "bob"}, "created": T0 - 2 * DAY,
         "text": "Also reproduced on Firefox.", "deleted": False, "visibility": None},
        {"id": "4-318", "author": {"login": "alice"}, "created": T0 - DAY,
         "text": "Raising the timeout to 30 s fixes it; PR incoming.", "deleted": False, "visibility": None},
    ],
}
PROJECTS = [{"id": "0-7", "shortName": "DEMO", "name": "Demo"}]
NEXT_ID = [58]


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        sys.stderr.write("mock: " + fmt % args + "\n")

    def send(self, body, status=200):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def parts(self):
        url = urlparse(self.path)
        path = [unquote(p) for p in url.path.split("/") if p]
        skip = 0
        for kv in url.query.split("&"):
            k, _, v = kv.partition("=")
            if unquote(k) == "$skip":
                skip = int(v or 0)
        return path, skip

    def do_GET(self):
        path, skip = self.parts()
        if skip:
            return self.send([])
        match path:
            case ["api", "issues"]:
                return self.send(ISSUES)
            case ["api", "issues", iid]:
                found = [i for i in ISSUES if i["idReadable"] == iid]
                return self.send(found[0]) if found else self.send({"error": "Not Found"}, 404)
            case ["api", "issues", iid, "links"]:
                return self.send(LINKS.get(iid, []))
            case ["api", "issues", iid, "comments"]:
                return self.send(COMMENTS.get(iid, []))
            case ["api", "admin", "projects"]:
                return self.send(PROJECTS)
        self.send({"error": "Not Found"}, 404)

    def do_POST(self):
        path, _ = self.parts()
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
        match path:
            case ["api", "issues"]:
                iid = f"DEMO-{NEXT_ID[0]}"
                NEXT_ID[0] += 1
                ISSUES.append(issue(int(iid[5:]), body.get("summary", ""), "Open", "Normal", age=0))
                return self.send({"idReadable": iid})
            case ["api", "issues", iid, "comments"]:
                return self.send({"id": "4-320", "text": body.get("text", "")})
        self.send({"error": "Not Found"}, 404)


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 18431
    HTTPServer(("127.0.0.1", port), Handler).serve_forever()
