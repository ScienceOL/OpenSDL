package storage

import repo "github.com/scienceol/opensdl/service/pkg/repo"

type GetStorageTokenReq struct {
	Scene    repo.SceneType `form:"scene" json:"scene" binding:"required"`
	FileName string         `form:"filename" json:"filename" binding:"required"`
	SubPath  string         `form:"sub_path" json:"sub_path"`
}

type GetStorageTokenResp struct {
	URL     string `json:"url"`     // The pre-signed URL for uploading
	Path    string `json:"path"`    // The object path in the bucket
	Expires int64  `json:"expires"` // Expiration timestamp
}
