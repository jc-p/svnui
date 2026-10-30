"""真实 Codex CLI 隔离回归：只请求本机临时 Responses 替身，不使用真实模型或凭据。"""
from __future__ import annotations

import http.server
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import unittest

ROOT = Path(__file__).resolve().parents[1]


class CodexWire(unittest.TestCase):
    def test_real_cli_has_no_tools_project_docs_mcp_or_notify(self) -> None:
        codex = os.environ.get('SVNUI_CODEX_TEST_BIN') or shutil.which('codex')
        binary = Path(os.environ.get('SVNUI_TEST_BIN', str(ROOT / 'target/debug/svnui')))
        if not codex or not binary.is_file():
            self.skipTest('需要本机 Codex CLI 和当前源码构建的 svnui；不自动安装或构建')
        captured: list[dict[str, object]] = []

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, format: str, *args: object) -> None:
                pass

            def do_POST(self) -> None:
                size = int(self.headers.get('Content-Length', '0'))
                if not 0 < size <= 2 * 1024 * 1024:
                    self.send_error(413)
                    return
                body = json.loads(self.rfile.read(size))
                captured.append(body)
                content = json.dumps({'messages': ['OK']})
                item = {'id': 'msg_test', 'type': 'message', 'role': 'assistant',
                        'content': [{'type': 'output_text', 'text': content, 'annotations': []}]}
                events = [
                    {'type': 'response.created', 'response': {'id': 'resp_test', 'status': 'in_progress', 'output': []}},
                    {'type': 'response.output_item.added', 'output_index': 0,
                     'item': {'id': 'msg_test', 'type': 'message', 'role': 'assistant', 'content': []}},
                    {'type': 'response.output_text.delta', 'item_id': 'msg_test',
                     'output_index': 0, 'content_index': 0, 'delta': content},
                    {'type': 'response.output_item.done', 'output_index': 0, 'item': item},
                    {'type': 'response.completed', 'response': {'id': 'resp_test', 'status': 'completed',
                     'output': [item], 'usage': {'input_tokens': 10, 'output_tokens': 5, 'total_tokens': 15}}},
                ]
                response = ''.join('event: ' + event['type'] + '\ndata: ' + json.dumps(event) + '\n\n'
                                   for event in events).encode()
                self.send_response(200)
                self.send_header('Content-Type', 'text/event-stream')
                self.send_header('Content-Length', str(len(response)))
                self.end_headers()
                self.wfile.write(response)

        with tempfile.TemporaryDirectory(prefix='svnui cli wire ') as directory:
            root = Path(directory)
            home = root / 'codex home'
            home.mkdir()
            project = root / 'project'
            project.mkdir()
            (project / 'AGENTS.md').write_text('PROJECT_DOC_MUST_NOT_BE_SENT')
            (home / 'AGENTS.md').write_text('Global user rule: only generate the requested message.')
            marker = root / 'unexpected-tool-startup'
            server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            try:
                (home / 'config.toml').write_text(
                    'model_provider="svnui_test"\nmodel="gpt-5.1-codex"\n'
                    f'notify=["touch",{json.dumps(str(marker))}]\n'
                    '[features]\nenable_request_compression=false\n'
                    '[model_providers.svnui_test]\nname="svnui test"\n'
                    f'base_url="http://127.0.0.1:{server.server_port}/v1"\n'
                    'wire_api="responses"\nenv_key="SVNUI_CODEX_TEST_KEY"\nrequires_openai_auth=false\n'
                    '[mcp_servers."name.with.dots"]\ncommand="touch"\n'
                    f'args=[{json.dumps(str(marker))}]\n')
                config = root / 'svnui.toml'
                config.write_text('[llm]\nenabled=true\nprovider="codex"\ntimeout_secs=30\n'
                                  f'codex_bin={json.dumps(codex)}\nsend_diff=false\n')
                result = subprocess.run([str(binary.resolve()), 'llm', '--test'], cwd=project,
                                        env=dict(os.environ, CODEX_HOME=str(home), SVNUI_CONFIG=str(config),
                                                 SVNUI_CODEX_TEST_KEY='synthetic-key'),
                                        text=True, capture_output=True, timeout=40)
                self.assertEqual(result.returncode, 0, result.stderr[:1000])
                self.assertEqual(len(captured), 1)
                self.assertFalse(captured[0].get('tools'), 'CLI 升级引入工具时必须复核隔离策略')
                context = json.dumps(captured[0], ensure_ascii=False)
                self.assertNotIn('PROJECT_DOC_MUST_NOT_BE_SENT', context)
                self.assertNotIn(str(project), context)
                self.assertNotIn('Available skills', context)
                self.assertFalse(marker.exists(), 'MCP 或通知命令不应启动')
                self.assertIn('回复: OK', result.stdout)
            finally:
                server.shutdown()
                server.server_close()
                thread.join(timeout=2)


if __name__ == '__main__':
    unittest.main()
