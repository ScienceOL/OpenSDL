package model

type StorageToken struct {
	BaseModel
	Token  string `gorm:"type:text;not null" json:"token"`
	Path   string `gorm:"type:text;not null" json:"path"`
	Scene  string `gorm:"type:varchar(255);not null;index:idx_storage_token_scene" json:"scene"`
	UserID string `gorm:"type:varchar(120);not null;index:idx_storage_token_user_id" json:"user_id"`
}

func (*StorageToken) TableName() string {
	return "storage_token"
}
