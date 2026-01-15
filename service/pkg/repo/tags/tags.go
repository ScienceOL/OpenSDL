package tags

import (
	// 外部依赖
	"context"

	clause "gorm.io/gorm/clause"
	
	// 内部引用
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	repo "github.com/scienceol/opensdl/service/pkg/repo"
	model "github.com/scienceol/opensdl/service/pkg/model"
)

type tagImpl struct {
	repo.IDOrUUIDTranslate
}

func NewTag() repo.Tags {
	return &tagImpl{
		IDOrUUIDTranslate: repo.NewBaseDB(),
	}
}

func (t *tagImpl) UpsertTags(ctx context.Context, tags []*model.Tags) error {
	if len(tags) == 0 {
		return nil
	}

	statement := t.DBWithContext(ctx).Clauses(clause.OnConflict{
		Columns: []clause.Column{
			{Name: "type"},
			{Name: "name"},
		},
		DoNothing: true,
	}).Create(tags)

	if statement.Error != nil {
		logger.Errorf(ctx, "UpsertTags err: %+v", statement.Error)
		return code.CreateDataErr.WithMsg(statement.Error.Error())
	}

	return nil
}

func (t *tagImpl) GetAllTags(ctx context.Context, tagType model.TagType) ([]string, error) {
	tags := []string{}
	if err := t.DBWithContext(ctx).
		Model(&model.Tags{}).
		Select("name").
		Where("type = ?", tagType).
		Find(&tags).Error; err != nil {
		logger.Errorf(ctx, "GetAllTags fail tag: %+v", tagType)

		return nil, code.QueryRecordErr.WithMsgf("tag: %+v", tagType)
	}

	return tags, nil
}
