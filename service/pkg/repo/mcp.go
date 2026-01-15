package repo

import (
	// 外部依赖
	"context"

	validator "github.com/go-playground/validator/v10"
)

var validate = validator.New()

// CallMCPRequest MCP 调用请求参数
type CallMCPRequest struct {
	Host string   `json:"host" validate:"required"`
	Path string   `json:"path" validate:"required"`
	Body string   `json:"body"` // JSON 字符串
	Urls []string `json:"urls,omitempty"`
}

// Validate 验证请求参数
func (r *CallMCPRequest) Validate() error {
	return validate.Struct(r)
}

type Mcp interface {
	CallMCP(ctx context.Context, req *CallMCPRequest) (map[string]any, error)
}
