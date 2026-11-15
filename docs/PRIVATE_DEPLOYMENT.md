# 개인용 배포

주소: https://autoforge.helloworld0822.site

이 배포는 세션 로그인으로 API, WebSocket, 업로드 이미지를 보호합니다.
로그인 요청은 nginx에서 빈도를 제한합니다. Redis는 AOF와 볼륨으로 프로젝트를
보관하고 RabbitMQ 및 산출물도 전용 볼륨을 사용합니다.

## 계정과 설정

서버의 `~/.config/autoforge/deployment.env`에 로그인 아이디, 비밀번호,
세션 서명 키와 RabbitMQ 비밀번호를 보관합니다. 파일 권한은 `0600`입니다.
이 파일을 Git에 추가하지 마세요.

현재 로그인 비밀번호를 서버 터미널에서 확인:

```bash
sed -n 's/^LOGIN_PASSWORD=//p' ~/.config/autoforge/deployment.env
```

`CURSOR_API_KEY`, Stitch 인증 정보 등 외부 서비스 연동 값은 이 파일에
별도로 설정해야 합니다. 설정 전에도 로그인과 대시보드를 사용할 수 있지만
관련 AI 파이프라인 단계는 실행할 수 없습니다.

## 배포와 재시작

저장소 루트에서 실행합니다. Docker Compose 2.24.4 이상이 필요합니다.

```bash
docker compose --env-file ~/.config/autoforge/deployment.env \
  -f compose.yml -f compose.private.yml up -d --build --wait
```

비밀번호를 변경할 때는 `LOGIN_PASSWORD`와 `SESSION_SECRET`을 함께 갱신하고
위 명령으로 재생성하면 기존 로그인 쿠키도 무효화됩니다. RabbitMQ 비밀번호는
초기화된 데이터 볼륨에서 환경 변수 변경만으로 갱신되지 않습니다.

nginx는 `127.0.0.1:18090`에만 바인딩합니다. 전용 Cloudflare Tunnel은
`~/.cloudflared/autoforge-config.yml`과 사용자 서비스
`cloudflared-autoforge.service`에서 관리합니다.

```bash
systemctl --user status cloudflared-autoforge
docker compose --env-file ~/.config/autoforge/deployment.env \
  -f compose.yml -f compose.private.yml ps
```

Docker의 재시작 정책과 사용자 systemd의 linger 설정으로 서버 재부팅 후에도
서비스를 복구합니다. `docker compose down -v`는 저장 데이터를 삭제하므로
재시작에 사용하지 마세요.
