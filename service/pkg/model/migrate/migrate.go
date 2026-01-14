package migrate

import (
	// 外部依赖
	"context"

	// 内部引用
	db "github.com/scienceol/opensdl/service/pkg/middleware/db"
	model "github.com/scienceol/opensdl/service/pkg/model"
	utils "github.com/scienceol/opensdl/service/pkg/utils"
)

func Table(_ context.Context) error {
	return utils.IfErrReturn(func() error {
		return db.DB().DBIns().AutoMigrate(
			&model.Laboratory{},             // 实验室
			&model.ResourceNodeTemplate{},   // 资源模板
			&model.ResourceHandleTemplate{}, // 资源 handle 模板
			&model.WorkflowNodeTemplate{},   // 实验室动作
			&model.MaterialNode{},           // 物料节点
			&model.MaterialEdge{},           // 物料边
			&model.Workflow{},               // 工作流表
			&model.WorkflowNode{},           // 工作流接电表
			&model.WorkflowEdge{},           // 工作流连线表
			&model.WorkflowHandleTemplate{}, // 工作流连线节点表
			&model.WorkflowNodeJob{},        // 工作流节点任务结果表
			&model.WorkflowTask{},           // 工作流任务表
			&model.Tags{},                   // tags 系统
			&model.LaboratoryMember{},       // 实验室成员
			&model.LaboratoryInvitation{},   // 实验室邀请记录
			&model.MaterialMachine{},        // 开发机器
			&model.Notebook{},               // 记录本表
			&model.NotebookGroup{},          // 记录本行记录表
			&model.NotebookParam{},          // 记录本节点参数表
			// &model.Sample{},                 // 样品记录表
			&model.WorkflowNodeJobSample{},  // 节点运行结果对应样品子结果
			&model.Reagent{},                // 试剂表
			&model.StorageToken{},           // 存储token表
			// &model.CustomRole{},             // 自定义角色
			// &model.CustomRolePerm{},         // 自定义角色权限
			// &model.PolicyResource{},         // 权限资源
			// &model.UserRole{},               // 用户角色
			// &model.Project{},                // 项目
		) // 动作节点handle 模板
	}, func() error {
		// 创建 gin 索引
		return db.DB().DBIns().Exec(`CREATE INDEX IF NOT EXISTS idx_workflow_tags ON workflow USING gin(tags) WHERE published = true;`).Error
	}, func() error {
		// 创建 gin 索引
		return db.DB().DBIns().Exec(`CREATE INDEX IF NOT EXISTS idx_resource_node_template_tags ON resource_node_template USING gin(tags);`).Error
	})
}
