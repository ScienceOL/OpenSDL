package repo

import "context"

type Sandbox interface {
	ExecCode(ctx context.Context, pyCode string, inputs map[string]any) (map[string]any, string, error)
}
