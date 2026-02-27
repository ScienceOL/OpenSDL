package migrate

import (
	"context"

	"github.com/scienceol/osdl/pkg/middleware/db"
	"github.com/scienceol/osdl/pkg/middleware/logger"
	"github.com/scienceol/osdl/pkg/repo/model"
)

func Table(ctx context.Context) error {
	d := db.DB().DBWithContext(ctx)
	models := []any{
		&model.Laboratory{},
		&model.LaboratoryMember{},
		&model.LaboratoryInvitation{},
		&model.CustomRole{},
		&model.UserRole{},
		&model.MaterialNode{},
		&model.MaterialEdge{},
		&model.ResourceNodeTemplate{},
		&model.ResourceHandleTemplate{},
		&model.WorkflowNodeTemplate{},
		&model.WorkflowHandleTemplate{},
		&model.Workflow{},
		&model.WorkflowNode{},
		&model.WorkflowEdge{},
		&model.WorkflowTask{},
		&model.WorkflowNodeJob{},
		&model.NotebookGroup{},
		&model.NotebookParam{},
		&model.NotebookSample{},
	}
	for _, m := range models {
		if err := d.AutoMigrate(m); err != nil {
			logger.Errorf(ctx, "migrate table err: %+v", err)
			return err
		}
	}
	return nil
}
