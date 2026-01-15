package repo

import (
	// 外部依赖
	"context"

	// 内部引用
	model "github.com/scienceol/opensdl/service/pkg/model"
)

type Tags interface {
	UpsertTags(ctx context.Context, tags []*model.Tags) error
	GetAllTags(ctx context.Context, tagType model.TagType) ([]string, error)
}
