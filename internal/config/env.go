package config

type AuthSource string

const (
	AuthCasdoor AuthSource = "casdoor"
	AuthBohr    AuthSource = "bohr"
)

type Auth struct {
	AuthSource AuthSource `mapstructure:"OAUTH_SOURCE" default:"casdoor"`
}

type RPC struct {
	Account  RPCAccount  `mapstructure:",squash"`
	Bohr     RPCBohr     `mapstructure:",squash"`
	BohrCore RPCBohrCore `mapstructure:",squash"`
}

type RPCAccount struct {
	Addr string `mapstructure:"ACCOUNT_ADDR" default:"http://127.0.0.1"`
}

type RPCBohr struct {
	Addr string `mapstructure:"BOHR_ADDR" default:"http://127.0.0.1"`
}

type RPCBohrCore struct {
	Addr string `mapstructure:"BOHR_CORE_ADDR" default:"http://127.0.0.1"`
}

type Database struct {
	Host     string `mapstructure:"DATABASE_HOST" default:"localhost"`
	Port     int    `mapstructure:"DATABASE_PORT" default:"5432"`
	Name     string `mapstructure:"DATABASE_NAME" default:"osdl"`
	User     string `mapstructure:"DATABASE_USER" default:"postgres"`
	Password string `mapstructure:"DATABASE_PASSWORD" default:"osdl"`
}

type Redis struct {
	Host     string `mapstructure:"REDIS_HOST" default:"127.0.0.1"`
	Port     int    `mapstructure:"REDIS_PORT" default:"6379"`
	Password string `mapstructure:"REDIS_PASSWORD"`
	DB       int    `mapstructure:"REDIS_DB" default:"0"`
}

type Server struct {
	Platform     string `mapstructure:"PLATFORM" default:"osdl"`
	Service      string `mapstructure:"SERVICE" default:"api"`
	Port         int    `mapstructure:"WEB_PORT" default:"8080"`
	SchedulePort int    `mapstructure:"SCHEDULE_PORT" default:"8081"`
	GrpcPort     int    `mapstructure:"GRPC_PORT" default:"9090"`
	Env          string `mapstructure:"ENV" default:"dev"`
}

type OAuth2 struct {
	ClientID     string   `mapstructure:"OAUTH2_CLIENT_ID"`
	ClientSecret string   `mapstructure:"OAUTH2_CLIENT_SECRET"`
	Scopes       []string `mapstructure:"OAUTH2_SCOPES" default:"[\"read\",\"write\",\"offline_access\"]"`
	Addr         string   `mapstructure:"CASDOOR_ADDR" default:"http://localhost:8000"`
	TokenURL     string   `mapstructure:"OAUTH2_TOKEN_URL" default:"http://localhost:8000/api/login/oauth/access_token"`
	AuthURL      string   `mapstructure:"OAUTH2_AUTH_URL" default:"http://localhost:8000/login/oauth/authorize"`
	RedirectURL  string   `mapstructure:"OAUTH2_REDIRECT_URL" default:"http://localhost:8080/api/auth/callback/casdoor"`
	UserInfoURL  string   `mapstructure:"OAUTH2_USERINFO_URL" default:"http://localhost:8000/api/get-account"`
}

type Log struct {
	LogPath  string `mapstructure:"LOG_PATH" default:"./info.log"`
	LogLevel string `mapstructure:"LOG_LEVEL" default:"info"`
}

type Trace struct {
	Version         string `mapstructure:"TRACE_VERSION" default:"0.0.1"`
	TraceEndpoint   string `mapstructure:"TRACE_TRACEENDPOINT" default:""`
	MetricEndpoint  string `mapstructure:"TRACE_METRICENDPOINT" default:""`
	TraceProject    string `mapstructure:"TRACE_TRACEPROJECT" default:""`
	TraceInstanceID string `mapstructure:"TRACE_TRACEINSTANCEID" default:""`
	TraceAK         string `mapstructure:"TRACE_TRACEAK" default:""`
	TraceSK         string `mapstructure:"TRACE_TRACESK" default:""`
}

type Job struct {
	JobQueueName string `mapstructure:"JOB_QUEUE_NAME" default:"osdl_workflow_job_queue"`
}

type Sandbox struct {
	Addr   string `mapstructure:"SANDBOX_ADDR" default:"http://127.0.0.1:8082"`
	ApiKey string `mapstructure:"SANDBOX_APIKEY" default:"osdl-sandbox"`
}
