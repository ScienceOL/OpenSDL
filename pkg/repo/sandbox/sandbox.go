package sandbox

import (
	"context"
	"net/http"

	"github.com/go-resty/resty/v2"
	"github.com/scienceol/osdl/internal/config"
	"github.com/scienceol/osdl/pkg/common/code"
	"github.com/scienceol/osdl/pkg/middleware/logger"
	"github.com/scienceol/osdl/pkg/repo"
)

type Data struct {
	Error  string `json:"error"`
	Stdout string `json:"stdout"`
}

type SandboxRet struct {
	Code    int    `json:"code"`
	Message string `json:"message"`
	Data    Data   `json:"data"`
}

type SandboxImpl struct {
	client *resty.Client
}

func NewSandbox() repo.Sandbox {
	sandboxConf := config.Global().Sandbox
	return &SandboxImpl{
		client: resty.New().
			EnableTrace().
			SetHeaders(map[string]string{
				"X-Api-Key":    sandboxConf.ApiKey,
				"Content-Type": "application/json",
			}).
			SetBaseURL(sandboxConf.Addr),
	}
}

func (s *SandboxImpl) ExecCode(ctx context.Context, pyCode string, inputs map[string]any) (map[string]any, string, error) {
	ret := &SandboxRet{}
	res, err := s.client.R().SetContext(ctx).
		SetBody(map[string]any{
			"language":       "python3",
			"code":           pyCode,
			"enable_network": true,
		}).
		SetResult(ret).Post("/api/v1/sandbox/run")
	if err != nil {
		logger.Errorf(ctx, "ExecCode post run code err: %+v", err)
		return nil, "", code.RPCHttpErr.WithErr(err)
	}
	if res.StatusCode() != http.StatusOK {
		return nil, "", code.RPCHttpCodeErr.WithMsgf("http code: %d", res.StatusCode())
	}
	if ret.Code != 0 {
		return nil, "", code.RPCHttpCodeErr.WithMsgf("code: %d", ret.Code)
	}
	if ret.Data.Error != "" {
		return nil, ret.Data.Error, code.ExecWorkflowNodeScriptErr.WithMsg(ret.Data.Error)
	}
	// TODO: implement proper response parsing
	return map[string]any{"stdout": ret.Data.Stdout}, "", nil
}
