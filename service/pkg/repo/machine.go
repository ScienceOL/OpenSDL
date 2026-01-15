package repo

import (
	// 外部依赖
	"context"

	// 内部引用
	model "github.com/scienceol/opensdl/service/pkg/model"
)

type Machine interface {
	DelMachine(ctx context.Context, req *model.DelMachineReq) error
	MachineStatus(ctx context.Context, req *model.MachineStatusReq) (*model.MachineStatusRes, error)
	StopMachine(ctx context.Context, req *model.StopMachineReq) error
	CreateMachine(ctx context.Context, req *model.CreateMachineReq) (int64, error)
	RestartMachine(ctx context.Context, req *model.RestartMachineReq) error
	JoinProject(ctx context.Context, req *model.JoninProjectReq) error
}
