# AutoForge 분산 모드. 초기 배치와 리소스 상한의 출처는 compose.yml이다.
backend = {
  workload = "PDF 업로드와 단계별 AI 작업 실행, 대시보드의 상태 조회 및 실시간 갱신. 업로드 기본 상한은 50 MiB. 운영 피크 RPS, 일일 프로젝트 수, 지연 목표는 미측정."
  components = {
    ingress = {
      role       = "nginx에서 정적 UI를 제공하고 API와 WebSocket을 프록시한다."
      capacity   = "Compose 초기 1개, CPU 상한 0.5개"
      depends_on = ["api"]
    }
    api = {
      role       = "Actix가 프로젝트 생성·조회와 산출물 다운로드를 처리하고 작업을 등록한다."
      capacity   = "Compose 초기 1개, CPU 상한 1개"
      depends_on = ["redis", "rabbitmq", "artifacts"]
    }
    orchestrator = {
      role       = "단계 완료 이벤트에 따라 프로젝트 상태를 갱신하고 다음 작업을 등록한다."
      capacity   = "Compose 초기 1개, CPU 상한 1개"
      depends_on = ["rabbitmq", "redis", "artifacts"]
    }
    worker = {
      role       = "계획서를 추출하고 OmniRoute와 디자인 서비스에 단계별 작업을 요청한다."
      capacity   = "Compose 워커당 동시 작업 4개. 증설 시 큐 대기 시간과 외부 API 한도를 함께 확인한다."
      depends_on = ["rabbitmq", "redis", "artifacts"]
    }
    rabbitmq = {
      role       = "단계 실행 명령과 결과 이벤트를 전달한다."
      capacity   = "Compose 초기 1개, 메모리 상한 512M"
      depends_on = []
    }
    redis = {
      role       = "프로젝트 상태, 목록 인덱스 및 실시간 알림 채널을 제공한다."
      capacity   = "Compose 초기 1개, 메모리 상한 256M. 레코드 수와 직렬화 크기에 따라 사용량을 측정한다."
      depends_on = []
    }
    artifacts = {
      role       = "PDF, 설계 HCL, 명세, 보고서를 프로젝트별 키로 공유 볼륨에 보존한다."
      capacity   = "Compose 공유 볼륨 1개. 디스크 용량 산정에는 업로드 크기와 보존 기간이 필요하다."
      depends_on = []
    }
  }
  indexes = [
    {
      component  = "redis"
      query      = "프로젝트 상세와 단일 프로젝트 이벤트 처리"
      strategy   = "autoforge:project:{uuid} 키로 GET"
      validation = "상세 조회당 GET 1회와 응답 크기, p95 지연을 확인한다."
    },
    {
      component  = "redis"
      query      = "대시보드와 전체 프로젝트 구독의 목록 조회"
      strategy   = "저장 시 레코드 SET과 autoforge:index:projects의 SADD를 같은 트랜잭션으로 처리한다. 목록은 SMEMBERS 후 최대 128개 키씩 MGET한다."
      validation = "N개 프로젝트에 SMEMBERS 1회와 MGET ceil(N/128)회를 확인한다. 저장당 SADD 비용과 인덱스 메모리를 함께 측정한다."
    }
  ]
  comparisons = {
    slack = {
      component  = "worker"
      decision   = "adopt"
      reason     = "긴 작업의 스케줄링과 실행을 분리한다. AutoForge는 기존 RabbitMQ와 워커로 이 원리를 적용할 수 있다."
      validation = "큐 대기 시간, 워커 점유율, 외부 API 대기 시간을 분리 측정하고 증설 효과를 확인한다."
    }
    discord = {
      component  = "api"
      decision   = "defer"
      reason     = "목록 인덱스와 묶음 조회를 먼저 적용했다. 여러 구독자가 같은 상태를 동시에 읽는 빈도는 아직 측정하지 않았다."
      validation = "구독자 증가 시 이벤트당 중복 Redis 조회 수와 p95가 함께 증가하면 동일 조회 병합을 검토한다."
    }
  }
}

references = [
  {
    id        = "slack"
    published = "2023-09-28"
    source    = "https://slack.engineering/executing-cron-scripts-reliably-at-scale/"
    scale     = "공용 작업 큐에서 하루 약 90억 건의 작업 처리"
    technique = "스케줄러와 워커를 분리하고 기존 작업 큐와 중복 제거 테이블을 활용했다."
  },
  {
    id        = "discord"
    published = "2023-03-06"
    source    = "https://discord.com/blog/how-discord-stores-trillions-of-messages"
    scale     = "2022년 초 Cassandra 177개 노드에 메시지 수조 건 저장"
    technique = "동일 행에 대한 동시 조회를 데이터 서비스에서 병합했다."
  }
]
