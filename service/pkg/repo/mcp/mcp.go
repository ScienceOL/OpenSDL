package mcp

import (
	// 外部依赖
	"context"
	"encoding/json"
	"net/http"
	"net/url"
	"time"

	resty "github.com/go-resty/resty/v2"

	// 内部引用
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	repo "github.com/scienceol/opensdl/service/pkg/repo"
)

// MCPImpl 通过 HTTP 调用外部 MCP 服务
type MCPImpl struct {
	client *resty.Client
}

// NewMCP 创建一个 MCP 客户端
func NewMCP() repo.Mcp {
	return &MCPImpl{
		client: resty.New().
			EnableTrace().
			SetHeader("Content-Type", "application/json").
			SetTimeout(120 * time.Second),
	}
}

func (s *MCPImpl) CallMCP(ctx context.Context, req *repo.CallMCPRequest) (map[string]any, error) {
	if s.client == nil {
		return nil, code.RPCHttpErr.WithMsg("mcp client not initialized")
	}

	// 解析 body JSON 字符串并处理 urls
	body := make(map[string]any)
	if req.Body != "" {
		err := json.Unmarshal([]byte(req.Body), &body)
		if err != nil {
			logger.Warnf(ctx, "CallMCP failed to parse body JSON string: %s, err: %+v", req.Body, err)
		}
	}

	// 将 urls 添加到 body 中
	if len(req.Urls) > 0 {
		body["urls"] = req.Urls
	}

	// 拼接 URL
	base, err := url.Parse(req.Host)
	if err != nil {
		return nil, code.ParamErr.WithMsg("invalid host")
	}
	rel, err := url.Parse(req.Path)
	if err != nil {
		return nil, code.ParamErr.WithMsg("invalid path")
	}
	fullURL := base.ResolveReference(rel).String()

	result := make(map[string]any)

	// 发送 POST 请求
	resp, err := s.client.R().
		SetContext(ctx).
		SetBody(body).      // body 作为 JSON 请求体
		SetResult(&result). // 自动反序列化到 result
		Post(fullURL)
	if err != nil {
		logger.Errorf(ctx, "CallMCP post url: %s err: %+v, body: %+v", fullURL, err, body)
		return nil, code.RPCHttpErr.WithErr(err)
	}

	if resp.StatusCode() != http.StatusOK {
		logger.Errorf(ctx, "CallMCP http code not 200, url: %s, code: %d, body: %s",
			fullURL, resp.StatusCode(), resp.String())
		return nil, code.RPCHttpCodeErr.WithMsgf("CallMCP http code: %d", resp.StatusCode())
	}

	return result, nil
}
